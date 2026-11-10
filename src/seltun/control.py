"""The CLI's side of the control channel: one JSON request per line to the daemon's loopback port, one JSON answer back (``native/daemon/src/control.rs``).

Requests carry only a verb (``ping``, ``status``, ``reload``, ``shutdown``). ``reload`` makes the daemon re-read ``state.json`` and ``config.toml``, which is how ``seltun on`` / ``seltun off`` take effect.
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
    """Send ``cmd`` and return the daemon's status. ``timeout`` is generous: a reload that starts the TUN adapter takes a few seconds."""
    try:
        with socket.create_connection(address, timeout=timeout) as conn:
            conn.sendall(json.dumps({"cmd": cmd}).encode() + b"\n")
            line = conn.makefile("rb").readline()
    except ConnectionRefusedError, TimeoutError:
        raise DaemonNotRunning(
            f"the daemon does not answer on {address[0]}:{address[1]}; start it with `seltun start`"
        ) from None
    except OSError as exc:
        raise SeltunError(
            f"talking to the daemon on {address[0]}:{address[1]}: {exc}"
        ) from exc
    try:
        answer = json.loads(line)
    except ValueError:
        raise SeltunError(
            f"something that is not the seltun daemon answers on {address[0]}:{address[1]}"
        ) from None
    if not answer.get("ok"):
        raise SeltunError(f"the daemon refused: {answer.get('error')}")
    return answer["status"]
