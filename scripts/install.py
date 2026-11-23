#!/usr/bin/env -S uv run --script

# /// script
# requires-python = ">=3.14"
# dependencies = []
# ///
"""Install seltun from this checkout with `uv tool install`, on Windows and on macOS.

On macOS, the script also signs the daemon with a stable self-signed certificate. An ad-hoc signature identifies a binary by its hash, thus each build looks like a new program to macOS. The background item of launchd and each permission that macOS keeps for the daemon then belong to the old build. With a fixed certificate and identifier, the daemon keeps its identity after each new build. On the first run, the script creates the certificate in the login keychain. The build hook (`hatch_build.py`) signs with it, and this script signs the installed daemon again to be sure.

If autostart is installed, the script stops the daemon before the install and starts it again after the install, so that the new binary runs. No password is necessary.

Usage: uv run scripts/install.py [--no-install]
  --no-install  Do not install. Only sign the installed daemon (macOS) and restart it.
Environment: SELTUN_CERT is the name of the certificate (default "seltun Code Signing").
"""

import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parent.parent
CERT = os.environ.get("SELTUN_CERT", "seltun Code Signing")
IDENTIFIER = "io.github.cychen2021.seltund"
KEYCHAIN = Path.home() / "Library" / "Keychains" / "login.keychain-db"


def run(*cmd: str | Path, check: bool = True) -> subprocess.CompletedProcess[str]:
    print("+", " ".join(str(c) for c in cmd), flush=True)
    return subprocess.run([str(c) for c in cmd], check=check, text=True)


def has_cert() -> bool:
    found = subprocess.run(
        ["security", "find-certificate", "-c", CERT, str(KEYCHAIN)], capture_output=True
    )
    return found.returncode == 0


def create_cert() -> None:
    """Create the self-signed code-signing certificate in the login keychain."""
    print(
        f'Create the self-signed certificate "{CERT}" in the login keychain.',
        flush=True,
    )
    with tempfile.TemporaryDirectory() as tmp:
        d = Path(tmp)
        (d / "cert.cnf").write_text(
            f"""[req]
distinguished_name = dn
x509_extensions = ext
prompt = no
[dn]
CN = {CERT}
[ext]
basicConstraints = critical, CA:false
keyUsage = critical, digitalSignature
extendedKeyUsage = critical, codeSigning
""",
            encoding="utf-8",
        )
        # The system LibreSSL writes a PKCS#12 file that `security import` can read. OpenSSL 3 needs `-legacy`.
        openssl = "/usr/bin/openssl"
        subprocess.run(
            [
                openssl,
                "req",
                "-x509",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-days",
                "3650",
                "-config",
                d / "cert.cnf",
                "-keyout",
                d / "key.pem",
                "-out",
                d / "cert.pem",
            ],
            check=True,
            capture_output=True,
        )
        subprocess.run(
            [
                openssl,
                "pkcs12",
                "-export",
                "-inkey",
                d / "key.pem",
                "-in",
                d / "cert.pem",
                "-out",
                d / "cert.p12",
                "-passout",
                "pass:seltun",
            ],
            check=True,
        )
        # -T lets codesign use the key with no prompt.
        subprocess.run(
            [
                "security",
                "import",
                d / "cert.p12",
                "-k",
                KEYCHAIN,
                "-P",
                "seltun",
                "-T",
                "/usr/bin/codesign",
            ],
            check=True,
            stdout=subprocess.DEVNULL,
        )


def installed_daemon() -> Path:
    tool_dir = Path(
        subprocess.run(
            ["uv", "tool", "dir"], check=True, capture_output=True, text=True
        ).stdout.strip()
    )
    name = "seltund.exe" if sys.platform == "win32" else "seltund"
    for path in (tool_dir / "seltun").rglob(name):
        if (
            path.parent.name == "bin"
            and path.parent.parent.name == "seltun"
            and path.is_file()
        ):
            return path
    sys.exit(f"The installed daemon is not in {tool_dir / 'seltun'}.")


def sign(binary: Path) -> None:
    run("codesign", "--force", "--sign", CERT, "--identifier", IDENTIFIER, binary)
    shown = subprocess.run(
        ["codesign", "--display", "--requirements", "-", str(binary)],
        capture_output=True,
        text=True,
    )
    for line in (shown.stdout + shown.stderr).splitlines():
        if "designated" in line:
            print(line)


def autostart_installed(seltun: str | None) -> bool:
    if seltun is None:
        return False
    status = subprocess.run(
        [seltun, "status", "--json"], capture_output=True, text=True
    )
    try:
        return json.loads(status.stdout)["autostart"] is not None
    except ValueError, KeyError:
        return False


def main() -> None:
    args = sys.argv[1:]
    if args in (["-h"], ["--help"]):
        print(__doc__)
        return
    if args not in ([], ["--no-install"]):
        sys.exit(f"Unknown arguments: {' '.join(args)}. See --help.")
    install = not args
    if sys.platform not in ("win32", "darwin"):
        sys.exit("seltun works on Windows and macOS only.")
    if sys.platform == "darwin" and not has_cert():
        create_cert()
    # Stop the daemon first. On Windows, uv cannot replace a binary that runs.
    restart = autostart_installed(shutil.which("seltun"))
    if restart:
        run("seltun", "daemon", "stop", check=False)
    if install:
        # --reinstall-package makes uv build again, also when a cached wheel has the old signature.
        run(
            "uv",
            "tool",
            "install",
            "--force",
            "--reinstall-package",
            "seltun",
            PROJECT_ROOT,
        )
    if sys.platform == "darwin":
        sign(installed_daemon())
    if restart:
        # Autostart starts the daemon with administrator rights (root on macOS) and shows no prompt.
        run(shutil.which("seltun") or "seltun", "daemon", "start")
    else:
        print(
            "Autostart is not installed. To start the daemon, run `seltun daemon start`."
        )


if __name__ == "__main__":
    main()
