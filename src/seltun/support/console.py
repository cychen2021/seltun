"""User-facing status output, routed through loguru.

The ``print_*`` helpers keep one policy for the whole codebase: INFO and above go to stderr (stdout stays clean for command output such as ``status --json``), DEBUG only when ``--verbose`` is given.
"""

from __future__ import annotations

import sys

from loguru import logger as _loguru

# We manage every sink explicitly; drop loguru's default stderr sink.
_loguru.remove()
_sink_id: int | None = None


def configure(verbose: bool = False) -> None:
    """(Re)install the stderr sink at INFO, or DEBUG when ``verbose``."""
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
    """Log a step/progress message (INFO)."""
    _log("INFO", "STEP 🔄", message)


def print_debug(message: str) -> None:
    """Log a debug message (DEBUG, shown only with ``--verbose``)."""
    _log("DEBUG", "DEBG 🐛", message)
