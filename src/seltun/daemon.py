"""Find and start ``seltund``, the Rust daemon (``docs/daemon.md``).

The wheel has the binary and ``wintun.dll`` in ``seltun/bin/`` (``hatch_build.py``). In a source checkout, the fallback is ``native/target/release/``. The CLI gives the daemon absolute ``--config`` and ``--state`` paths. Thus the daemon does not depend on ``PATH`` or on the working directory.

The name of the executable is important: Clash sees each forwarded connection as a connection of the process ``seltund.exe``.
"""

from __future__ import annotations

import shutil
import subprocess
import sys
import time
from pathlib import Path

from .control import request
from .support.errors import DaemonNotRunning, SeltunError

__all__ = [
    "DAEMON_EXE",
    "command",
    "find_binary",
    "run",
    "start_background",
]

DAEMON_EXE = "seltund.exe" if sys.platform == "win32" else "seltund"

_PACKAGE = Path(__file__).resolve().parent
_CHECKOUT_BUILD = _PACKAGE.parents[1] / "native" / "target" / "release" / DAEMON_EXE


def find_binary() -> Path:
    for candidate in (_PACKAGE / "bin" / DAEMON_EXE, _CHECKOUT_BUILD):
        if candidate.is_file():
            return candidate
    if found := shutil.which(DAEMON_EXE):
        return Path(found)
    raise SeltunError(
        f"{DAEMON_EXE} is not found. Install with `uv tool install`. In a checkout, run `cargo build --release --manifest-path native/Cargo.toml`."
    )


def command(config: Path, state: Path, log: Path | None) -> list[str]:
    """The command line of the daemon for the given config file and state file. If ``log`` is set, the daemon adds its log to that file."""
    argv = [
        str(find_binary()),
        "--config",
        str(config.resolve()),
        "--state",
        str(state.resolve()),
    ]
    if log is not None:
        argv += ["--log-file", str(log)]
    return argv


def run(argv: list[str]) -> int:
    """Run the daemon in the foreground until it stops. Ctrl+C also reaches the daemon. Thus the code waits for the daemon to remove the adapter."""
    process = subprocess.Popen(argv)
    try:
        return process.wait()
    except KeyboardInterrupt:
        try:
            return process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            process.kill()
            return process.wait()


def start_background(argv: list[str], control: tuple[str, int], log: Path) -> int:
    """Start the daemon in a separate process and return its pid when its control port answers.

    The daemon gets its own process group. Thus the daemon continues when you close the terminal. On Windows, the daemon also leaves the job of this process, if the job permits it.

    The daemon must be elevated to create the TUN adapter. In a terminal that is not elevated, the daemon runs but cannot turn on. ``seltun status`` shows the reason. The elevated way is the scheduled task of ``seltun autostart enable``.
    """
    log.parent.mkdir(parents=True, exist_ok=True)
    options: dict = {
        "stdin": subprocess.DEVNULL,
        "stdout": subprocess.DEVNULL,
        "stderr": subprocess.DEVNULL,
    }
    if sys.platform == "win32":
        flags = subprocess.CREATE_NO_WINDOW | subprocess.CREATE_NEW_PROCESS_GROUP
        try:
            process = subprocess.Popen(
                argv,
                creationflags=flags | subprocess.CREATE_BREAKAWAY_FROM_JOB,
                **options,
            )
        except PermissionError:
            process = subprocess.Popen(argv, creationflags=flags, **options)
    else:
        process = subprocess.Popen(argv, start_new_session=True, **options)
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        if (code := process.poll()) is not None:
            raise SeltunError(
                f"The daemon stopped immediately (code {code}). Run `seltun daemon run` to see the reason, or read {log}."
            )
        try:
            request(control, "ping", timeout=1)
            return process.pid
        except DaemonNotRunning:
            time.sleep(0.2)
    raise SeltunError(
        f"The daemon (pid {process.pid}) does not answer on {control[0]}:{control[1]} after 10 seconds. See {log}."
    )
