"""Shared pytest configuration.

Typer shows help and error output with Rich. A narrow ``COLUMNS`` value from the environment, or a forced color mode, makes the substring assertions of ``CliRunner`` unstable. Thus this file sets both values for each test.
"""

import os

import pytest


def _pin_cli_output_env() -> None:
    os.environ["COLUMNS"] = "200"
    os.environ["TERMINAL_WIDTH"] = "200"
    os.environ.pop("FORCE_COLOR", None)
    os.environ.pop("CLICOLOR_FORCE", None)


_pin_cli_output_env()


@pytest.fixture(autouse=True)
def _stable_cli_output():
    """Set the width and color values again for each test (see the module docstring)."""
    _pin_cli_output_env()
