"""The locations of the seltun files. They are the per-user locations of the platform, not the working directory.

On Windows, the config file is in ``%APPDATA%/seltun`` (roaming, as for other user settings). The data are in ``%LOCALAPPDATA%/seltun``: the switch state and the log of the daemon. The options ``--config`` and ``--state`` change the first two locations.
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
    """The saved switch state (``{"enabled": bool}``)."""
    return platformdirs.user_data_path(APP, appauthor=False) / "state.json"


def log_dir() -> Path:
    return platformdirs.user_log_path(APP, appauthor=False)


def daemon_log() -> Path:
    """The log of the daemon. The daemon writes to it when it runs without a console (autostart, ``seltun daemon start``)."""
    return log_dir() / "seltund.log"
