"""Start the daemon at login with administrator rights.

The daemon needs administrator rights to create the Wintun adapter and its routes. A Run-key entry cannot get these rights without a UAC prompt at each login. Thus autostart is a Task Scheduler task. At the logon of the user, it starts ``seltund.exe`` with the highest privileges and shows no prompt.

To create or remove the task, use an elevated terminal one time. To start, stop, or query the task, you do not need an elevated terminal.

The name of the task is :data:`NAME`.
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
        raise SeltunError("Autostart works on Windows only.")
    return subprocess.run(
        ["schtasks", *args],
        capture_output=True,
        text=True,
        creationflags=subprocess.CREATE_NO_WINDOW,
    )


def _failure(result: subprocess.CompletedProcess[str]) -> SeltunError:
    return SeltunError(f"schtasks: {(result.stderr or result.stdout).strip()}")


def task_xml(user: str, argv: list[str]) -> str:
    """The task definition. The task starts at the logon of ``user`` with the highest privileges. It has no time limit, and it does not stop on battery power."""
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
    """Register the logon task for ``argv``. The code replaces an existing task. Return the command line that the task runs."""
    if not is_admin():
        raise SeltunError("To create the autostart task, use an elevated terminal.")
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
    """Remove the task. Return true if there was a task."""
    if registered() is None:
        return False
    if not is_admin():
        raise SeltunError("To remove the autostart task, use an elevated terminal.")
    result = _schtasks("/Delete", "/TN", NAME, "/F")
    if result.returncode != 0:
        raise _failure(result)
    return True


def registered() -> str | None:
    """The command line that the task runs, or ``None`` if there is no task."""
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
    """Start the task with administrator rights. No prompt shows."""
    result = _schtasks("/Run", "/TN", NAME)
    if result.returncode != 0:
        raise _failure(result)


def end() -> None:
    """Stop the instance of the task that runs."""
    _schtasks("/End", "/TN", NAME)
