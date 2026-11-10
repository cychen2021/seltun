"""Behavior config, read from ``config.toml`` in the per-user config directory (see :mod:`seltun.paths`); ``seltun init`` writes it from the packaged ``config.example.toml``.

Every key is optional; a missing file or table means the defaults below. A bare top-level key, or an unknown key in a table, is rejected so a typo cannot silently fall back to a default. The daemon reads the same file (``native/daemon/src/config.rs``) and applies the same defaults.
"""

from __future__ import annotations

import tomllib
from dataclasses import dataclass, field, fields
from ipaddress import IPv4Address
from pathlib import Path
from typing import Any

from .support.errors import SeltunError

__all__ = [
    "EXAMPLE_CONFIG",
    "Config",
    "DaemonConfig",
    "ProcessesConfig",
    "TunConfig",
    "UpstreamConfig",
]

#: The template ``seltun init`` copies; shipped inside the package.
EXAMPLE_CONFIG = Path(__file__).with_name("config.example.toml")


def _host_port(what: str, text: str) -> tuple[str, int]:
    host, sep, port = text.rpartition(":")
    if not sep or not port.isdigit():
        raise SeltunError(f"{what} must be host:port, got {text!r}")
    return host.strip("[]"), int(port)


@dataclass
class ProcessesConfig:
    #: Executable file names or full paths, case-insensitive.
    names: list[str] = field(default_factory=list)


@dataclass
class UpstreamConfig:
    address: str = "127.0.0.1:7897"
    protocol: str = "socks5"
    udp: bool = False

    def validate(self) -> None:
        _host_port("[upstream].address", self.address)
        if self.protocol not in ("socks5", "http"):
            raise SeltunError(
                f'[upstream].protocol must be "socks5" or "http", got {self.protocol!r}'
            )
        if self.udp and self.protocol != "socks5":
            raise SeltunError('[upstream].udp needs protocol = "socks5"')


@dataclass
class TunConfig:
    name: str = "seltun"
    address: str = "198.19.0.1"
    mtu: int = 1500

    def validate(self) -> None:
        try:
            ip = IPv4Address(self.address)
        except ValueError:
            raise SeltunError(
                f"[tun].address must be an IPv4 address, got {self.address!r}"
            ) from None
        if ip.is_loopback or ip.is_unspecified:
            raise SeltunError("[tun].address must not be loopback or unspecified")


@dataclass
class DaemonConfig:
    """Where the CLI reaches the daemon (and where the daemon binds)."""

    control: str = "127.0.0.1:7898"

    @property
    def host_port(self) -> tuple[str, int]:
        return _host_port("[daemon].control", self.control)


@dataclass
class Config:
    processes: ProcessesConfig = field(default_factory=ProcessesConfig)
    upstream: UpstreamConfig = field(default_factory=UpstreamConfig)
    tun: TunConfig = field(default_factory=TunConfig)
    daemon: DaemonConfig = field(default_factory=DaemonConfig)

    @classmethod
    def load(cls, path: Path) -> Config:
        if not path.exists():
            return cls()
        try:
            raw = tomllib.loads(path.read_text(encoding="utf-8"))
        except tomllib.TOMLDecodeError as exc:
            raise SeltunError(f"{path}: {exc}") from exc
        config = cls()
        for key, value in raw.items():
            if not isinstance(value, dict) or key not in {f.name for f in fields(cls)}:
                raise SeltunError(f"{path}: unknown key or table {key!r}")
            setattr(config, key, _table(getattr(config, key), key, value, path))
        config.upstream.validate()
        config.tun.validate()
        config.daemon.host_port  # noqa: B018 -- validates
        return config


def _table(default: Any, name: str, raw: dict[str, Any], path: Path) -> Any:
    """Fill the dataclass ``default`` from a TOML table; kebab-case keys map to snake_case fields."""
    known = {f.name for f in fields(default)}
    values: dict[str, Any] = {}
    for key, value in raw.items():
        attr = key.replace("-", "_")
        if attr not in known:
            raise SeltunError(f"{path}: unknown key {key!r} in [{name}]")
        values[attr] = value
    return type(default)(**values)
