"""Where seltun keeps its files: the platform's per-user locations, not the working directory.

On Windows, config is in ``%APPDATA%/seltun`` (roaming, like other user settings) and data -- the on/off state, the daemon's log -- in ``%LOCALAPPDATA%/seltun``. ``--config`` and ``--state`` override the first two.
"""

from __future__ import annotations

from pathlib import Path

import platformdirs

__all__ = ["config_file", "daemon_log", "log_dir", "state_file"]

APP = "seltun"


def config_file() -> Path:
    return (
        platformdirs.user_config_path(APP, appauthor=False, roaming=True)
        / "config.toml"
    )


def state_file() -> Path:
    """The persisted on/off switch (``{"enabled": bool}``)."""
    return platformdirs.user_data_path(APP, appauthor=False) / "state.json"


def log_dir() -> Path:
    return platformdirs.user_log_path(APP, appauthor=False)


def daemon_log() -> Path:
    """Where the daemon logs when it runs without a console (autostart, ``seltun start``)."""
    return log_dir() / "seltund.log"
