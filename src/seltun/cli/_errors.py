"""Shared error handling of the CLI.

A command body wraps its work in :func:`handle_errors`. The function logs an expected :class:`~seltun.support.errors.SeltunError` and ends the command with a clean non-zero exit code. Thus each command does not need its own ``try/except``.
"""

from __future__ import annotations

from contextlib import contextmanager
from typing import Iterator

import typer

from ..support.console import print_error
from ..support.errors import SeltunError

__all__ = ["handle_errors"]


@contextmanager
def handle_errors() -> Iterator[None]:
    """Log a :class:`SeltunError` and exit with a non-zero code. Other errors continue up."""
    try:
        yield
    except SeltunError as exc:
        print_error(str(exc))
        raise typer.Exit(1)
