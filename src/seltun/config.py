"""Config file. seltun reads ``config.toml`` from the config directory of the user (see :mod:`seltun.paths`). ``seltun init`` creates it from ``config.example.toml``.

All keys are optional. If the file or a table is missing, seltun uses the defaults below. seltun rejects a key at the top level and an unknown key in a table. Thus a typo cannot select a default without a warning. The daemon reads the same file (``native/daemon/src/config.rs``) and uses the same defaults.
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

#: The template that ``seltun init`` copies. The template is in the root of the repository. The wheel build (``hatch_build.py``) copies it into the package. Thus an installed seltun finds it next to this file, and a checkout finds it in the root.
_PACKAGED = Path(__file__).with_name("config.example.toml")
EXAMPLE_CONFIG = (
    _PACKAGED
    if _PACKAGED.is_file()
    else Path(__file__).parents[2] / "config.example.toml"
)


def _host_port(what: str, text: str) -> tuple[str, int]:
    host, sep, port = text.rpartition(":")
    if not sep or not port.isdigit():
        raise SeltunError(
            f"{what} must have the form host:port. The value is {text!r}."
        )
    return host.strip("[]"), int(port)


@dataclass
class ProcessesConfig:
    #: File names of executables, or full paths. Case does not matter.
    names: list[str] = field(default_factory=list)


@dataclass
class UpstreamConfig:
    address: str = "127.0.0.1:1080"
    protocol: str = "socks5"
    udp: bool = False

    def validate(self) -> None:
        _host_port("[upstream].address", self.address)
        if self.protocol not in ("socks5", "http"):
            raise SeltunError(
                f'[upstream].protocol must be "socks5" or "http". The value is {self.protocol!r}.'
            )
        if self.udp and self.protocol != "socks5":
            raise SeltunError('[upstream].udp = true needs protocol = "socks5".')


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
                f"[tun].address must be an IPv4 address. The value is {self.address!r}."
            ) from None
        if ip.is_loopback or ip.is_unspecified:
            raise SeltunError(
                "[tun].address must not be a loopback address or an unspecified address."
            )


@dataclass
class DaemonConfig:
    """The address that the CLI uses to reach the daemon. The daemon binds the same address."""

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
            raw = tomllib.loads(path.read_text(encoding="utf-8-sig"))
        except tomllib.TOMLDecodeError as exc:
            raise SeltunError(f"{path}: {exc}") from exc
        config = cls()
        for key, value in raw.items():
            if not isinstance(value, dict) or key not in {f.name for f in fields(cls)}:
                raise SeltunError(f"{path}: the key or table {key!r} is not known.")
            setattr(config, key, _table(getattr(config, key), key, value, path))
        config.upstream.validate()
        config.tun.validate()
        config.daemon.host_port  # noqa: B018 -- validates
        return config


def _table(default: Any, name: str, raw: dict[str, Any], path: Path) -> Any:
    """Fill the dataclass ``default`` from a TOML table. A key in kebab-case maps to a field in snake_case."""
    known = {f.name for f in fields(default)}
    values: dict[str, Any] = {}
    for key, value in raw.items():
        attr = key.replace("-", "_")
        if attr not in known:
            raise SeltunError(f"{path}: the key {key!r} in [{name}] is not known.")
        values[attr] = value
    return type(default)(**values)
