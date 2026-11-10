"""The installed version. It comes from ``pyproject.toml``. The daemon (``seltund --version``) uses the version of the Cargo workspace. Change both together."""

from importlib.metadata import PackageNotFoundError, version

try:
    __version__ = version("seltun")
except PackageNotFoundError:  # running from a source tree that was never installed
    __version__ = "0.0.0+unknown"
