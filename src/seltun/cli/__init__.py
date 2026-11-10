"""The ``seltun`` command-line interface (Typer).

This layer is thin. It loads the config and the state, writes the switch state, and sends commands to the daemon (:mod:`seltun.control`, :mod:`seltun.daemon`, :mod:`seltun.autostart`).
"""

from __future__ import annotations

import io
import json
import shutil
import sys
import time
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Any

import typer

from .. import autostart as autostart_
from .. import daemon as daemon_
from .. import paths
from .._version import __version__
from ..config import EXAMPLE_CONFIG, Config
from ..control import request
from ..drivers import ensure_wintun
from ..state import State
from ..support import console
from ..support.console import (
    print_info,
    print_step,
    print_success,
    print_warning,
)
from ..support.errors import DaemonNotRunning, SeltunError
from ._errors import handle_errors

app = typer.Typer(add_completion=False, no_args_is_help=True)
autostart_app = typer.Typer(
    no_args_is_help=True,
    help="Start the daemon at login with an elevated Task Scheduler task.",
)
app.add_typer(autostart_app, name="autostart")
switch_app = typer.Typer(
    no_args_is_help=True,
    help="Set the switch to on or off. The switch keeps its state after a restart.",
)
app.add_typer(switch_app, name="switch")
daemon_app = typer.Typer(
    no_args_is_help=True,
    help="Run, start, or stop the daemon.",
)
app.add_typer(daemon_app, name="daemon")


@dataclass
class Context:
    config: Config
    config_path: Path
    state_path: Path

    @property
    def control(self) -> tuple[str, int]:
        return self.config.daemon.host_port


def _show_version(value: bool) -> None:
    if value:
        typer.echo(f"seltun {__version__}")
        raise typer.Exit()


@app.callback()
def _root(
    ctx: typer.Context,
    config: Path | None = typer.Option(
        None,
        "--config",
        help=f"Config file (TOML). Default: {paths.config_file()}",
        show_default=False,
    ),
    state: Path | None = typer.Option(
        None,
        "--state",
        help=f"Switch state file. Default: {paths.state_file()}",
        show_default=False,
    ),
    verbose: bool = typer.Option(False, "--verbose", "-v", help="Show debug messages."),
    _version: bool = typer.Option(
        False,
        "--version",
        callback=_show_version,
        is_eager=True,
        help="Show the version and stop.",
    ),
) -> None:
    """Steer selected processes into a TUN adapter and forward them to Clash."""
    console.configure(verbose)
    config_path = config or paths.config_file()
    with handle_errors():
        ctx.obj = Context(
            config=Config.load(config_path),
            config_path=config_path,
            state_path=state or paths.state_file(),
        )


@app.command()
def init(ctx: typer.Context) -> None:
    """Create the config file if it is missing. Download wintun.dll next to the daemon if it is missing. You can run this command again with no risk."""
    c: Context = ctx.obj
    with handle_errors():
        if c.config_path.exists():
            print_info(f"{c.config_path} is already there. No change.")
        else:
            c.config_path.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(EXAMPLE_CONFIG, c.config_path)
            print_success(f"Created {c.config_path}")
            print_info(
                "In [processes].names, write the processes to steer. Then run `seltun switch on`."
            )
        target = daemon_.find_binary().parent
        if (target / "wintun.dll").is_file():
            print_info(f"{target / 'wintun.dll'} is already there. No change.")
        else:
            print_step(f"Download Wintun to {target}")
            try:
                dll = ensure_wintun(target)
            except (OSError, RuntimeError) as exc:
                raise SeltunError(f"Download of Wintun failed: {exc}") from exc
            print_success(f"Created {dll}")


@app.command("paths")
def show_paths(ctx: typer.Context) -> None:
    """Show the locations of the config file, the switch state, and the daemon log."""
    c: Context = ctx.obj
    for label, path in (
        ("config", c.config_path),
        ("state", c.state_path),
        ("log", paths.daemon_log()),
    ):
        typer.echo(f"{label:<8} {path}")


def _save_state(c: Context, enabled: bool) -> None:
    State(enabled=enabled).save(c.state_path)


def _summary(status: dict[str, Any]) -> str:
    """One line about the state of the daemon."""
    if status["active"]:
        return f"on. The daemon steers {', '.join(status['processes'])} to Clash at {status['upstream']}"
    return "on, but the daemon does not steer" if status["enabled"] else "off"


def _apply(c: Context, enabled: bool) -> None:
    """Save the switch. Then make the daemon follow it. A daemon that is not running reads the state when it starts."""
    with handle_errors():
        _save_state(c, enabled)
        if enabled and not c.config.processes.names:
            print_warning(
                f"[processes].names is empty in {c.config_path}. seltun has no process to steer."
            )
        try:
            status = request(c.control, "reload")
        except DaemonNotRunning:
            print_info(
                f"Saved: seltun is {'on' if enabled else 'off'}. The daemon is not running. The switch applies when the daemon starts (`seltun daemon start`)."
            )
            return
        if status["error"]:
            raise SeltunError(
                f"{status['error']} The switch stays {'on' if enabled else 'off'}. Correct the problem, then run `seltun reload`."
            )
    print_success(f"seltun is {_summary(status)}")


@switch_app.command("on")
def switch_on(ctx: typer.Context) -> None:
    """Set the switch to on. It stays on when the daemon or the computer starts again."""
    _apply(ctx.obj, True)


@switch_app.command("off")
def switch_off(ctx: typer.Context) -> None:
    """Set the switch to off. The daemon removes the TUN adapter. The switch stays off when the daemon or the computer starts again."""
    _apply(ctx.obj, False)


@app.command()
def reload(ctx: typer.Context) -> None:
    """Make the daemon read the config file and the switch state again."""
    c: Context = ctx.obj
    with handle_errors():
        status = request(c.control, "reload")
        if status["error"]:
            raise SeltunError(status["error"])
    print_success(f"seltun is {_summary(status)}")


@app.command()
def status(
    ctx: typer.Context,
    as_json: bool = typer.Option(False, "--json", help="Show machine-readable output."),
) -> None:
    """Show the saved switch, the state of the daemon, and the startup task."""
    c: Context = ctx.obj
    with handle_errors():
        persisted = State.load(c.state_path)
        try:
            daemon = request(c.control, "status")
        except DaemonNotRunning:
            daemon = None
        startup = autostart_.registered()
    if as_json:
        typer.echo(
            json.dumps(
                {"state": asdict(persisted), "daemon": daemon, "startup": startup},
                indent=2,
            )
        )
        return
    typer.echo(
        f"switch   {'on' if persisted.enabled else 'off'} (saved in {c.state_path})"
    )
    if daemon is None:
        typer.echo("daemon   not running (`seltun daemon start`)")
    else:
        typer.echo(
            f"daemon   running, pid {daemon['pid']}{'' if daemon['elevated'] else ' (not elevated: cannot create the TUN adapter)'}"
        )
        typer.echo(f"steering {'active' if daemon['active'] else 'inactive'}")
        typer.echo(f"upstream {daemon['upstream']}")
        typer.echo(f"selected {', '.join(daemon['processes']) or '(none)'}")
        if daemon["error"]:
            typer.echo(f"problem  {daemon['error']}")
    typer.echo(f"startup  {_autostart_summary(startup)}")


@daemon_app.command("start")
def daemon_start(ctx: typer.Context) -> None:
    """Start the daemon. If the startup task is installed, the task starts the daemon with administrator rights."""
    c: Context = ctx.obj
    with handle_errors():
        try:
            request(c.control, "ping", timeout=2)
        except DaemonNotRunning:
            pass
        else:
            print_info("The daemon is already running")
            return
        if autostart_.registered() is not None:
            autostart_.run()
            deadline = time.monotonic() + 15
            while True:
                try:
                    status = request(c.control, "reload", timeout=30)
                    break
                except DaemonNotRunning:
                    if time.monotonic() > deadline:
                        raise SeltunError(
                            f"The startup task ran, but the daemon does not answer. See {paths.daemon_log()}."
                        ) from None
                    time.sleep(0.3)
            print_success(
                f"The startup task started the daemon. seltun is {_summary(status)}"
            )
            return
        argv = daemon_.command(c.config_path, c.state_path, paths.daemon_log())
        pid = daemon_.start_background(argv, c.control, paths.daemon_log())
        if not autostart_.is_admin():
            print_warning(
                "This terminal is not elevated. The daemon cannot create the TUN adapter. Run `seltun autostart enable` one time in an elevated terminal. After that, `seltun daemon start` starts the daemon with administrator rights."
            )
    print_success(f"The daemon (pid {pid}) runs. The log is {paths.daemon_log()}")


@daemon_app.command("stop")
def daemon_stop(ctx: typer.Context) -> None:
    """Stop the daemon. The switch keeps its state."""
    c: Context = ctx.obj
    with handle_errors():
        try:
            request(c.control, "shutdown")
        except DaemonNotRunning:
            print_info("The daemon was not running")
            return
    print_success("The daemon stops.")


@daemon_app.command("run")
def daemon_run(ctx: typer.Context) -> None:
    """Run the daemon in the foreground until you press Ctrl+C. To turn on, use an elevated terminal."""
    c: Context = ctx.obj
    with handle_errors():
        argv = daemon_.command(c.config_path, c.state_path, None)
    raise typer.Exit(daemon_.run(argv))


@autostart_app.command("enable")
def autostart_enable(ctx: typer.Context) -> None:
    """Start the daemon at each login with administrator rights. Run this command one time in an elevated terminal."""
    c: Context = ctx.obj
    with handle_errors():
        line = autostart_.enable(
            daemon_.command(c.config_path, c.state_path, paths.daemon_log())
        )
    print_success(f"The daemon starts at login: {line}")
    print_info("To start the daemon, run `seltun daemon start`.")


@autostart_app.command("disable")
def autostart_disable() -> None:
    """Do not start the daemon at login. A daemon that runs does not stop."""
    with handle_errors():
        removed = autostart_.disable()
    if removed:
        print_success("The daemon does not start at login.")
    else:
        print_info("The startup task was not installed.")


def _autostart_summary(line: str | None) -> str:
    return "disabled" if line is None else f"enabled: {line}"


def main() -> None:
    # The status prefixes use emoji. A legacy Windows console code page (cp1252) cannot encode them.
    for stream in (sys.stdout, sys.stderr):
        if isinstance(stream, io.TextIOWrapper):
            stream.reconfigure(encoding="utf-8")
    app()
