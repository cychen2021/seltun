"""Shared CLI error handling.

Command bodies wrap their work in :func:`handle_errors` so an expected :class:`~seltun.support.errors.SeltunError` is logged and turned into a clean non-zero exit, instead of repeating the same ``try/except`` in every command.
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
    """Log a :class:`SeltunError` and exit non-zero, leaving other errors to propagate."""
    try:
        yield
    except SeltunError as exc:
        print_error(str(exc))
        raise typer.Exit(1)
