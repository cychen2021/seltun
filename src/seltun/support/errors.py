"""Shared error types."""

__all__ = ["SeltunError", "DaemonNotRunning"]


class SeltunError(RuntimeError):
    """Base class for the failures that the user can expect.

    The CLI layer ends the command with a clean non-zero exit code. Internal callers can catch it as they catch each other exception.
    """


class DaemonNotRunning(SeltunError):
    """No program answers on the control address of the daemon."""
