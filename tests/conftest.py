"""Shared pytest configuration.

Typer renders help and error output with Rich; a narrow inherited ``COLUMNS`` or a forced color mode makes ``CliRunner`` substring assertions unstable, so both are pinned here for every test.
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
    """Re-pin the width/color env for every test (see the module docstring)."""
    _pin_cli_output_env()
