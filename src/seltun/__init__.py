"""seltun: steer selected processes into a TUN adapter and forward them to a local proxy.

The command-line interface is in the :mod:`seltun.cli` package. ``main`` is the entry point of the console script. The daemon is the Rust binary ``seltund`` (``native/daemon/``).
"""

from ._version import __version__
from .cli import app, main

__all__ = ["__version__", "app", "main"]
