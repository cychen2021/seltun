"""The installed version, from ``pyproject.toml``; the daemon (``seltund --version``) is built from the same workspace version."""

from importlib.metadata import PackageNotFoundError, version

try:
    __version__ = version("seltun")
except PackageNotFoundError:  # running from a source tree that was never installed
    __version__ = "0.0.0+unknown"
