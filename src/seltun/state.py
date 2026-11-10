"""Persistent switch state. The daemon also reads it.

``seltun switch on`` and ``seltun switch off`` write the file. The daemon reads it at startup and at each ``reload``. Thus the switch keeps its state when the daemon or the computer starts again. If the file is missing, the switch is off.
"""

from __future__ import annotations

import json
import os
from dataclasses import asdict, dataclass, fields
from pathlib import Path

__all__ = ["State"]


@dataclass
class State:
    #: Whether the selected processes are steered into the TUN adapter.
    enabled: bool = False

    @classmethod
    def load(cls, path: Path) -> State:
        if not path.exists():
            return cls()
        raw = json.loads(path.read_text(encoding="utf-8"))
        known = {f.name for f in fields(cls)}
        return cls(**{k: v for k, v in raw.items() if k in known})

    def save(self, path: Path) -> None:
        path.parent.mkdir(parents=True, exist_ok=True)
        tmp = path.with_suffix(".tmp")
        tmp.write_text(json.dumps(asdict(self), indent=2), encoding="utf-8")
        os.replace(tmp, path)
