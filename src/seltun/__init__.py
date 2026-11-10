"""seltun — steer selected processes into a TUN adapter and forward them to Clash.

The command-line interface lives in the :mod:`seltun.cli` package; ``main`` is the console-script entry point. The daemon itself is the Rust binary ``seltund`` (``native/daemon/``).
"""

from .cli import app, main

__all__ = ["app", "main"]
