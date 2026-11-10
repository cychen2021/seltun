"""Starting the daemon at login, elevated.

The daemon needs administrator rights (creating the Wintun adapter and its routes), and a Run-key entry cannot elevate without a UAC prompt at every login. So autostart is a Task Scheduler task that runs ``seltund.exe`` at the user's logon with the highest privileges, no prompt: the same arrangement Clash Verge's service mode achieves with a service. Creating or removing the task needs an elevated terminal once; starting, stopping and querying it does not.

The task name is :data:`NAME`.
"""

from __future__ import annotations

import ctypes
import getpass
import os
import subprocess
import sys
import tempfile
from pathlib import Path
from xml.sax.saxutils import escape

from .support.errors import SeltunError

__all__ = [
    "NAME",
    "disable",
    "enable",
    "end",
    "is_admin",
    "registered",
    "run",
    "task_xml",
]

NAME = "seltund"


def is_admin() -> bool:
    if sys.platform != "win32":
        return False
    return bool(ctypes.windll.shell32.IsUserAnAdmin())  # type: ignore[attr-defined]


def _schtasks(*args: str) -> subprocess.CompletedProcess[str]:
    if sys.platform != "win32":
        raise SeltunError("autostart is implemented on Windows only")
    return subprocess.run(
        ["schtasks", *args],
        capture_output=True,
        text=True,
        creationflags=subprocess.CREATE_NO_WINDOW,
    )


def _failure(result: subprocess.CompletedProcess[str]) -> SeltunError:
    return SeltunError(f"schtasks: {(result.stderr or result.stdout).strip()}")


def task_xml(user: str, argv: list[str]) -> str:
    """The task definition: at ``user``'s logon, highest privileges, no time limit, not stopped on battery."""
    program, *arguments = argv
    joined = subprocess.list2cmdline(arguments)
    return f"""<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.4" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo><Description>seltun daemon: steers selected processes into a TUN adapter.</Description></RegistrationInfo>
  <Triggers><LogonTrigger><Enabled>true</Enabled><UserId>{escape(user)}</UserId></LogonTrigger></Triggers>
  <Principals><Principal id="Author"><UserId>{escape(user)}</UserId><LogonType>InteractiveToken</LogonType><RunLevel>HighestAvailable</RunLevel></Principal></Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <Enabled>true</Enabled>
  </Settings>
  <Actions Context="Author"><Exec><Command>{escape(program)}</Command><Arguments>{escape(joined)}</Arguments></Exec></Actions>
</Task>
"""


def enable(argv: list[str]) -> str:
    """Register (or replace) the logon task for ``argv``; returns the command line it runs."""
    if not is_admin():
        raise SeltunError(
            "creating the startup task needs an elevated terminal (run it once as administrator)"
        )
    domain = os.environ.get("USERDOMAIN", "")
    user = f"{domain}\\{getpass.getuser()}" if domain else getpass.getuser()
    with tempfile.TemporaryDirectory() as tmp:
        definition = Path(tmp) / "task.xml"
        definition.write_text(task_xml(user, argv), encoding="utf-16")
        result = _schtasks("/Create", "/TN", NAME, "/XML", str(definition), "/F")
    if result.returncode != 0:
        raise _failure(result)
    return subprocess.list2cmdline(argv)


def disable() -> bool:
    """Remove the task; returns whether there was one."""
    if registered() is None:
        return False
    if not is_admin():
        raise SeltunError("removing the startup task needs an elevated terminal")
    result = _schtasks("/Delete", "/TN", NAME, "/F")
    if result.returncode != 0:
        raise _failure(result)
    return True


def registered() -> str | None:
    """The command line the task runs, or ``None`` if there is no task."""
    result = _schtasks("/Query", "/TN", NAME, "/XML")
    if result.returncode != 0:
        return None
    text = result.stdout
    if "<Command>" not in text:
        return ""
    command = text.split("<Command>")[1].split("</Command>")[0]
    arguments = (
        text.split("<Arguments>")[1].split("</Arguments>")[0]
        if "<Arguments>" in text
        else ""
    )
    return f"{command} {arguments}".strip()


def run() -> None:
    """Start the task now, elevated, without a prompt."""
    result = _schtasks("/Run", "/TN", NAME)
    if result.returncode != 0:
        raise _failure(result)


def end() -> None:
    """Stop the task's running instance."""
    _schtasks("/End", "/TN", NAME)
