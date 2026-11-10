"""The side of the CLI for the control channel. The CLI sends one JSON request in one line to the loopback port of the daemon. The daemon sends one JSON answer (``native/daemon/src/control.rs``).

A request has only a verb: ``ping``, ``status``, ``reload`` or ``shutdown``. The verb ``reload`` makes the daemon read ``state.json`` and ``config.toml`` again. This is how ``seltun switch on`` and ``seltun switch off`` take effect.
"""

from __future__ import annotations

import json
import socket
from typing import Any

from .support.errors import DaemonNotRunning, SeltunError

__all__ = ["request"]


def request(
    address: tuple[str, int], cmd: str, timeout: float = 30.0
) -> dict[str, Any]:
    """Send ``cmd`` and return the status of the daemon. The default ``timeout`` is long, because a reload that starts the TUN adapter takes a few seconds."""
    try:
        with socket.create_connection(address, timeout=timeout) as conn:
            conn.sendall(json.dumps({"cmd": cmd}).encode() + b"\n")
            line = conn.makefile("rb").readline()
    except ConnectionRefusedError, TimeoutError:
        raise DaemonNotRunning(
            f"The daemon does not answer on {address[0]}:{address[1]}. Start it with `seltun daemon start`."
        ) from None
    except OSError as exc:
        raise SeltunError(
            f"Communication with the daemon on {address[0]}:{address[1]} failed: {exc}"
        ) from exc
    try:
        answer = json.loads(line)
    except ValueError:
        raise SeltunError(
            f"A program that is not the seltun daemon answers on {address[0]}:{address[1]}."
        ) from None
    if not answer.get("ok"):
        raise SeltunError(f"The daemon refused the request: {answer.get('error')}")
    return answer["status"]
