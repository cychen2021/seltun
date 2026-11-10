"""Status output for the user, through loguru.

The ``print_*`` helpers give one policy for the whole codebase. INFO and higher levels go to stderr, so stdout has only the output of commands (for example ``status --json``). DEBUG shows only with ``--verbose``.
"""

from __future__ import annotations

import sys

from loguru import logger as _loguru

# We manage every sink explicitly; drop loguru's default stderr sink.
_loguru.remove()
_sink_id: int | None = None


def configure(verbose: bool = False) -> None:
    """Install the stderr sink again. The level is INFO, or DEBUG if ``verbose`` is true."""
    global _sink_id
    if _sink_id is not None:
        _loguru.remove(_sink_id)
        _sink_id = None
    if sys.stderr is None:
        # Without a console there is no stderr; stay silent.
        return
    _sink_id = _loguru.add(
        sys.stderr,
        level="DEBUG" if verbose else "INFO",
        format="{extra[header]} {message}",
        colorize=False,
    )


def _log(level: str, header: str, message: str) -> None:
    if _sink_id is None:
        configure()
    _loguru.bind(header=header).log(level, message)


def print_success(message: str) -> None:
    """Log a success message (INFO)."""
    _log("INFO", "SUCC ✅", message)


def print_error(message: str) -> None:
    """Log an error message (ERROR)."""
    _log("ERROR", "ERRO ❌", message)


def print_warning(message: str) -> None:
    """Log a warning message (WARNING)."""
    _log("WARNING", "WARN ⚠️", message)


def print_info(message: str) -> None:
    """Log an info message (INFO)."""
    _log("INFO", "INFO ℹ️", message)


def print_step(message: str) -> None:
    """Log a message about a step (INFO)."""
    _log("INFO", "STEP 🔄", message)


def print_debug(message: str) -> None:
    """Log a debug message (DEBUG). It shows only with ``--verbose``."""
    _log("DEBUG", "DEBG 🐛", message)
