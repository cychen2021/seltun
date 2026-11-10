"""Finding and launching ``seltund``, the Rust daemon (``docs/daemon.md``).

The wheel ships the binary and ``wintun.dll`` at ``seltun/bin/`` (``hatch_build.py``); a source checkout falls back to ``native/target/release/``. The daemon is told absolute ``--config`` / ``--state`` paths, so it depends on neither ``PATH`` nor the working directory. Its executable name is also what Clash sees: every connection it forwards comes from the process ``seltund.exe``.
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
        f"{DAEMON_EXE} not found: install with `uv tool install`, or in a checkout run `cargo build --release --manifest-path native/Cargo.toml`"
    )


def command(config: Path, state: Path, log: Path | None) -> list[str]:
    """The daemon's command line against the given config and state; with ``log`` it appends its log there."""
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
    """Run the daemon in the foreground until it exits; Ctrl+C reaches it too, so give it a moment to tear the adapter down."""
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
    """Start the daemon detached from this console and return its pid once its control port answers.

    It gets its own process group, so closing the terminal does not take it down; on Windows it also leaves this process's job when the job allows that. It needs to be elevated to create the TUN adapter, so from a normal terminal it runs but cannot turn on (`seltun status` says why); the scheduled task of `seltun autostart enable` is the elevated way.
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
                f"the daemon exited at once ({code}); run `seltun daemon run` to see why, or check {log}"
            )
        try:
            request(control, "ping", timeout=1)
            return process.pid
        except DaemonNotRunning:
            time.sleep(0.2)
    raise SeltunError(
        f"the daemon (pid {process.pid}) does not answer on {control[0]}:{control[1]} after 10 s; see {log}"
    )
