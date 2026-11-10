"""The ``seltun`` command-line interface (Typer).

This is the thin command layer: it loads config and state, writes the on/off state, and talks to the daemon (:mod:`seltun.control`, :mod:`seltun.daemon`, :mod:`seltun.autostart`).
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
    help="Start the daemon at login (an elevated Task Scheduler task).",
)
app.add_typer(autostart_app, name="autostart")
switch_app = typer.Typer(
    no_args_is_help=True,
    help="Turn steering on or off (persisted across restarts).",
)
app.add_typer(switch_app, name="switch")
daemon_app = typer.Typer(
    no_args_is_help=True,
    help="Run, start or stop the daemon.",
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
        help=f"Behavior config (TOML). Default: {paths.config_file()}",
        show_default=False,
    ),
    state: Path | None = typer.Option(
        None,
        "--state",
        help=f"On/off state file. Default: {paths.state_file()}",
        show_default=False,
    ),
    verbose: bool = typer.Option(False, "--verbose", "-v", help="Log debug detail."),
    _version: bool = typer.Option(
        False,
        "--version",
        callback=_show_version,
        is_eager=True,
        help="Show the version and exit.",
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
    """Create the config from the example if it is missing, and download wintun.dll (the TUN driver Clash Verge uses too) next to the daemon if it is missing. Safe to run again."""
    c: Context = ctx.obj
    with handle_errors():
        if c.config_path.exists():
            print_info(f"{c.config_path} already exists; left as it is")
        else:
            c.config_path.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(EXAMPLE_CONFIG, c.config_path)
            print_success(f"Wrote {c.config_path}")
            print_info(
                "List the processes to steer under [processes].names, then `seltun switch on`"
            )
        target = daemon_.find_binary().parent
        if (target / "wintun.dll").is_file():
            print_info(f"{target / 'wintun.dll'} already exists; left as it is")
        else:
            print_step(f"Fetching Wintun into {target}")
            try:
                dll = ensure_wintun(target)
            except (OSError, RuntimeError) as exc:
                raise SeltunError(f"fetching Wintun: {exc}") from exc
            print_success(f"Wrote {dll}")


@app.command("paths")
def show_paths(ctx: typer.Context) -> None:
    """Show where the config, the on/off state and the daemon log live."""
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
    """One line on what the daemon is doing relative to what was asked."""
    if status["active"]:
        return f"on: steering {', '.join(status['processes'])} to Clash at {status['upstream']}"
    return "on, but not steering" if status["enabled"] else "off"


def _apply(c: Context, enabled: bool) -> None:
    """Persist the switch, then make a running daemon follow it; a daemon that is not running picks the state up when it starts."""
    with handle_errors():
        _save_state(c, enabled)
        if enabled and not c.config.processes.names:
            print_warning(
                f"[processes].names is empty in {c.config_path}: nothing would be steered"
            )
        try:
            status = request(c.control, "reload")
        except DaemonNotRunning:
            print_info(
                f"Saved: seltun is {'on' if enabled else 'off'}. The daemon is not running; the switch applies when it starts (`seltun daemon start`)"
            )
            return
        if status["error"]:
            raise SeltunError(
                f"{status['error']} (the switch stays {'on' if enabled else 'off'}; fix it, then `seltun reload`)"
            )
    print_success(f"seltun is {_summary(status)}")


@switch_app.command("on")
def switch_on(ctx: typer.Context) -> None:
    """Turn steering on; it stays on across restarts of the daemon and the machine."""
    _apply(ctx.obj, True)


@switch_app.command("off")
def switch_off(ctx: typer.Context) -> None:
    """Turn steering off (the TUN adapter goes away); it stays off across restarts."""
    _apply(ctx.obj, False)


@app.command()
def reload(ctx: typer.Context) -> None:
    """Make the daemon re-read the config and the on/off state."""
    c: Context = ctx.obj
    with handle_errors():
        status = request(c.control, "reload")
        if status["error"]:
            raise SeltunError(status["error"])
    print_success(f"seltun is {_summary(status)}")


@app.command()
def status(
    ctx: typer.Context,
    as_json: bool = typer.Option(False, "--json", help="Machine-readable output."),
) -> None:
    """Show the persisted switch and what the daemon is doing."""
    c: Context = ctx.obj
    with handle_errors():
        persisted = State.load(c.state_path)
        try:
            daemon = request(c.control, "status")
        except DaemonNotRunning:
            daemon = None
    if as_json:
        typer.echo(json.dumps({"state": asdict(persisted), "daemon": daemon}, indent=2))
        return
    typer.echo(
        f"switch   {'on' if persisted.enabled else 'off'} (persisted in {c.state_path})"
    )
    if daemon is None:
        typer.echo("daemon   not running (`seltun daemon start`)")
        return
    typer.echo(
        f"daemon   running, pid {daemon['pid']}{'' if daemon['elevated'] else ' (not elevated: cannot create the TUN adapter)'}"
    )
    typer.echo(f"steering {'active' if daemon['active'] else 'inactive'}")
    typer.echo(f"upstream {daemon['upstream']}")
    typer.echo(f"selected {', '.join(daemon['processes']) or '(none)'}")
    if daemon["error"]:
        typer.echo(f"problem  {daemon['error']}")
    typer.echo(f"startup  {_autostart_summary(autostart_.registered())}")


@daemon_app.command("start")
def daemon_start(ctx: typer.Context) -> None:
    """Start the daemon (through its startup task if there is one, which runs it elevated)."""
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
                            f"the startup task ran but the daemon does not answer; see {paths.daemon_log()}"
                        ) from None
                    time.sleep(0.3)
            print_success(
                f"The daemon started through its startup task; seltun is {_summary(status)}"
            )
            return
        argv = daemon_.command(c.config_path, c.state_path, paths.daemon_log())
        pid = daemon_.start_background(argv, c.control, paths.daemon_log())
        if not autostart_.is_admin():
            print_warning(
                "This terminal is not elevated, so the daemon cannot create the TUN adapter; run `seltun autostart enable` from an elevated terminal once, and `seltun daemon start` will run it elevated from then on"
            )
    print_success(f"The daemon (pid {pid}) is running; it logs to {paths.daemon_log()}")


@daemon_app.command("stop")
def daemon_stop(ctx: typer.Context) -> None:
    """Stop the daemon (the persisted switch is left as it is)."""
    c: Context = ctx.obj
    with handle_errors():
        try:
            request(c.control, "shutdown")
        except DaemonNotRunning:
            print_info("The daemon was not running")
            return
    print_success("The daemon is stopping")


@daemon_app.command("run")
def daemon_run(ctx: typer.Context) -> None:
    """Run the daemon in the foreground until Ctrl+C (needs an elevated terminal to turn on)."""
    c: Context = ctx.obj
    with handle_errors():
        argv = daemon_.command(c.config_path, c.state_path, None)
    raise typer.Exit(daemon_.run(argv))


@autostart_app.command("enable")
def autostart_enable(ctx: typer.Context) -> None:
    """Start the daemon at every login, elevated (run once from an elevated terminal)."""
    c: Context = ctx.obj
    with handle_errors():
        line = autostart_.enable(
            daemon_.command(c.config_path, c.state_path, paths.daemon_log())
        )
    print_success(f"The daemon starts at login: {line}")
    print_info("To start it now, run `seltun daemon start`")


@autostart_app.command("disable")
def autostart_disable() -> None:
    """Stop starting the daemon at login (a running daemon keeps running)."""
    with handle_errors():
        removed = autostart_.disable()
    if removed:
        print_success("The daemon no longer starts at login")
    else:
        print_info("The daemon was not set to start at login")


@autostart_app.command("status")
def autostart_status() -> None:
    """Show whether the daemon starts at login, and with what command."""
    with handle_errors():
        line = autostart_.registered()
    typer.echo(_autostart_summary(line))


def _autostart_summary(line: str | None) -> str:
    return "disabled" if line is None else f"enabled: {line}"


def main() -> None:
    # Status prefixes are emoji; a legacy Windows console codepage (cp1252) cannot encode them.
    for stream in (sys.stdout, sys.stderr):
        if isinstance(stream, io.TextIOWrapper):
            stream.reconfigure(encoding="utf-8")
    app()
