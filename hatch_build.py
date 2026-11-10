"""Hatch build hook: compile ``seltund`` and ship it, with ``wintun.dll``, inside the wheel.

The daemon is a Rust binary (``native/daemon/``). A regular wheel build runs ``cargo build --release`` and places the binary and the pinned Wintun driver DLL (plus `config.example.toml`) at ``seltun/bin/``, where ``seltun`` looks first; the wheel is tagged for this platform. An editable install (``uv sync``) skips all of that: ``seltun`` then falls back to ``native/target/release/``, which ``cargo build --release`` keeps current, and ``seltun setup`` fetches the DLL next to it.
"""

from __future__ import annotations

import importlib.util
import os
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any

from hatchling.builders.hooks.plugin.interface import BuildHookInterface

DAEMON_EXE = "seltund.exe" if sys.platform == "win32" else "seltund"


def _load_drivers(root: Path) -> Any:
    """``seltun/drivers.py`` is stdlib-only, so the hook can load it without installing the package."""
    spec = importlib.util.spec_from_file_location(
        "seltun_drivers", root / "src" / "seltun" / "drivers.py"
    )
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class CustomBuildHook(BuildHookInterface):
    def initialize(self, version: str, build_data: dict[str, Any]) -> None:
        if self.target_name != "wheel" or version == "editable":
            return
        root = Path(self.root)
        manifest = root / "native" / "Cargo.toml"
        subprocess.run(
            [
                "cargo",
                "build",
                "--release",
                "--locked",
                "--package",
                "seltund",
                "--manifest-path",
                str(manifest),
            ],
            check=True,
        )
        target = Path(os.environ.get("CARGO_TARGET_DIR") or root / "native" / "target")
        build_data["force_include"][str(root / "config.example.toml")] = (
            "seltun/config.example.toml"
        )
        build_data["force_include"][str(target / "release" / DAEMON_EXE)] = (
            f"seltun/bin/{DAEMON_EXE}"
        )
        if sys.platform == "win32":
            staging = Path(tempfile.mkdtemp(prefix="seltun-wintun-"))
            dll = _load_drivers(root).ensure_wintun(staging)
            build_data["force_include"][str(dll)] = "seltun/bin/wintun.dll"
        build_data["pure_python"] = False
        build_data["infer_tag"] = True
