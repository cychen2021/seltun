"""Shared error types."""

__all__ = ["SeltunError", "DaemonNotRunning"]


class SeltunError(RuntimeError):
    """Base class for expected, user-facing failures.

    The CLI layer maps this to a clean non-zero exit; internal callers can catch it like any other exception.
    """


class DaemonNotRunning(SeltunError):
    """Nothing answers on the daemon's control address."""
