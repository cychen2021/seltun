"""Fetching ``wintun.dll``, the one third-party binary the daemon needs next to ``seltund.exe``.

Wintun (https://www.wintun.net) is the TUN driver Clash Verge's TUN mode uses too. The release zip is pinned by SHA-256, so a changed download is refused. Stdlib only: the build hook (``hatch_build.py``) loads this file directly.
"""

from __future__ import annotations

import hashlib
import io
import platform
import urllib.request
import zipfile
from pathlib import Path

__all__ = ["WINTUN_SHA256", "WINTUN_URL", "ensure_wintun"]

WINTUN_URL = "https://www.wintun.net/builds/wintun-0.14.1.zip"
WINTUN_SHA256 = "07c256185d6ee3652e09fa55c0b673e2624b565e02c4b9091c79ca7d2f24ef51"

_ARCH = {"AMD64": "amd64", "x86_64": "amd64", "ARM64": "arm64", "x86": "x86"}


def ensure_wintun(directory: Path, force: bool = False) -> Path:
    """Put ``wintun.dll`` in ``directory`` (if it is not there yet) and return its path."""
    target = directory / "wintun.dll"
    if target.is_file() and not force:
        return target
    arch = _ARCH.get(platform.machine())
    if arch is None:
        raise RuntimeError(f"no Wintun build for {platform.machine()}")
    with urllib.request.urlopen(WINTUN_URL, timeout=60) as response:
        data = response.read()
    if hashlib.sha256(data).hexdigest() != WINTUN_SHA256:
        raise RuntimeError(
            f"{WINTUN_URL} does not match the pinned SHA-256; refusing it"
        )
    with zipfile.ZipFile(io.BytesIO(data)) as archive:
        dll = archive.read(f"wintun/bin/{arch}/wintun.dll")
    directory.mkdir(parents=True, exist_ok=True)
    target.write_bytes(dll)
    return target
