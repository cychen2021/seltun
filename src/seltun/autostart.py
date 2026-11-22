"""Start the daemon at login with administrator rights.

The daemon needs administrator rights (root on macOS) to create the TUN adapter and its routes.

On Windows, a Run-key entry cannot get these rights without a UAC prompt at each login. Thus autostart is a Task Scheduler task. At the logon of the user, it starts ``seltund.exe`` with the highest privileges and shows no prompt. To create or remove the task, use an elevated terminal one time. To start, stop, or query the task, you do not need an elevated terminal. The name of the task is :data:`NAME`.

On macOS, autostart is a launchd daemon (:data:`LAUNCHD_PLIST`). It starts ``seltund`` as root when the computer starts. A sudoers rule (:data:`SUDOERS_FILE`) lets the user start the daemon with no password. To create or remove them, ``sudo`` asks for the password one time.
"""

from __future__ import annotations

import ctypes
import getpass
import os
import plistlib
import shlex
import subprocess
import sys
import tempfile
from pathlib import Path
from xml.sax.saxutils import escape

from .support.errors import SeltunError

__all__ = [
    "ELEVATION_HINT",
    "LAUNCHD_LABEL",
    "LAUNCHD_PLIST",
    "NAME",
    "SUDOERS_FILE",
    "disable",
    "enable",
    "end",
    "is_admin",
    "registered",
    "run",
    "launchd_plist",
    "sudoers_rule",
    "task_xml",
]

NAME = "seltund"
LAUNCHD_LABEL = "io.github.cychen2021.seltund"
LAUNCHD_PLIST = Path("/Library/LaunchDaemons") / f"{LAUNCHD_LABEL}.plist"
SUDOERS_FILE = Path("/etc/sudoers.d/seltund")
_LAUNCHCTL = "/bin/launchctl"
_SERVICE = f"system/{LAUNCHD_LABEL}"

if sys.platform == "darwin":
    ELEVATION_HINT = "The daemon does not run as root. It cannot create the TUN adapter. Run `seltun autostart enable` one time. After that, `seltun daemon start` starts the daemon as root with no password."
else:
    ELEVATION_HINT = "This terminal is not elevated. The daemon cannot create the TUN adapter. Run `seltun autostart enable` one time in an elevated terminal. After that, `seltun daemon start` starts the daemon with administrator rights."


def is_admin() -> bool:
    if sys.platform == "darwin":
        return os.geteuid() == 0
    if sys.platform != "win32":
        return False
    return bool(ctypes.windll.shell32.IsUserAnAdmin())  # type: ignore[attr-defined]


def _schtasks(*args: str) -> subprocess.CompletedProcess[str]:
    if sys.platform != "win32":
        raise SeltunError("Autostart works on Windows and macOS only.")
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


def _enable_windows(argv: list[str]) -> str:
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


def _disable_windows() -> bool:
    """Remove the task. Return true if there was a task."""
    if registered() is None:
        return False
    if not is_admin():
        raise SeltunError("To remove the autostart task, use an elevated terminal.")
    result = _schtasks("/Delete", "/TN", NAME, "/F")
    if result.returncode != 0:
        raise _failure(result)
    return True


def _registered_windows() -> str | None:
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


def _run_windows() -> None:
    """Start the task with administrator rights. No prompt shows."""
    result = _schtasks("/Run", "/TN", NAME)
    if result.returncode != 0:
        raise _failure(result)


def _end_windows() -> None:
    """Stop the instance of the task that runs."""
    _schtasks("/End", "/TN", NAME)


def launchd_plist(argv: list[str]) -> bytes:
    """The launchd daemon definition. launchd starts ``argv`` as root when the computer starts. It starts the daemon again if the daemon fails, but not after a stop with ``seltun daemon stop``."""
    return plistlib.dumps(
        {
            "Label": LAUNCHD_LABEL,
            "ProgramArguments": argv,
            "RunAtLoad": True,
            "KeepAlive": {"SuccessfulExit": False},
            "ProcessType": "Background",
        }
    )


def sudoers_rule(user: str) -> str:
    """The sudoers rule that lets ``user`` start (and restart) the launchd daemon with no password. The rule permits only these two exact commands."""
    return (
        f"{user} ALL=(root) NOPASSWD: {_LAUNCHCTL} kickstart {_SERVICE}, "
        f"{_LAUNCHCTL} kickstart -k {_SERVICE}\n"
    )


def _sudo(*args: str) -> subprocess.CompletedProcess[str]:
    """Run a command as root. ``sudo`` asks for the password in the terminal if it must."""
    return subprocess.run(["sudo", *args], capture_output=True, text=True)


def _check(result: subprocess.CompletedProcess[str], what: str) -> None:
    if result.returncode != 0:
        raise SeltunError(f"{what}: {(result.stderr or result.stdout).strip()}")


def _enable_macos(argv: list[str]) -> str:
    user = getpass.getuser()
    with tempfile.TemporaryDirectory() as tmp:
        plist = Path(tmp) / "daemon.plist"
        plist.write_bytes(launchd_plist(argv))
        sudoers = Path(tmp) / "sudoers"
        sudoers.write_text(sudoers_rule(user), encoding="utf-8")
        _check(_sudo("/usr/sbin/visudo", "-cf", str(sudoers)), "visudo")
        _check(
            _sudo(
                "/usr/bin/install",
                "-m",
                "440",
                "-o",
                "root",
                "-g",
                "wheel",
                str(sudoers),
                str(SUDOERS_FILE),
            ),
            "install",
        )
        # Remove the old daemon definition first. launchd does not read a changed file of a loaded daemon.
        _sudo(_LAUNCHCTL, "bootout", _SERVICE)
        _check(
            _sudo(
                "/usr/bin/install",
                "-m",
                "644",
                "-o",
                "root",
                "-g",
                "wheel",
                str(plist),
                str(LAUNCHD_PLIST),
            ),
            "install",
        )
        _check(
            _sudo(_LAUNCHCTL, "bootstrap", "system", str(LAUNCHD_PLIST)),
            "launchctl bootstrap",
        )
    return shlex.join(argv)


def _disable_macos() -> bool:
    if not LAUNCHD_PLIST.exists():
        return False
    _sudo(_LAUNCHCTL, "bootout", _SERVICE)
    _check(_sudo("/bin/rm", "-f", str(LAUNCHD_PLIST), str(SUDOERS_FILE)), "rm")
    return True


def _registered_macos() -> str | None:
    try:
        with LAUNCHD_PLIST.open("rb") as file:
            definition = plistlib.load(file)
    except FileNotFoundError:
        return None
    except OSError, plistlib.InvalidFileException:
        return ""
    return shlex.join(definition.get("ProgramArguments", []))


def _run_macos() -> None:
    # The sudoers rule permits this command with no password. If the rule is missing, sudo asks for the password.
    _check(_sudo(_LAUNCHCTL, "kickstart", _SERVICE), "launchctl kickstart")


def _end_macos() -> None:
    _sudo(_LAUNCHCTL, "kill", "TERM", _SERVICE)


def enable(argv: list[str]) -> str:
    """Register the autostart entry for ``argv``. The code replaces an existing entry. Return the command line that the entry runs. On macOS, the daemon also starts immediately."""
    return _enable_macos(argv) if sys.platform == "darwin" else _enable_windows(argv)


def disable() -> bool:
    """Remove the autostart entry. Return true if there was an entry."""
    return _disable_macos() if sys.platform == "darwin" else _disable_windows()


def registered() -> str | None:
    """The command line that the autostart entry runs, or ``None`` if there is no entry."""
    return _registered_macos() if sys.platform == "darwin" else _registered_windows()


def run() -> None:
    """Start the daemon with administrator rights. No prompt shows."""
    if sys.platform == "darwin":
        _run_macos()
    else:
        _run_windows()


def end() -> None:
    """Stop the instance of the daemon that the autostart entry started."""
    if sys.platform == "darwin":
        _end_macos()
    else:
        _end_windows()
