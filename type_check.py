#!/usr/bin/env -S uv run --script

# /// script
# requires-python = ">=3.12"
# dependencies = [
#     "pyright",
# ]
# ///
"""Type-check the Python sources with pyright. Lint the Rust workspace with clippy.

The script finds `src/`, `tests/`, the `*.py` files in the top directory, and `scripts/`. Clippy runs on `native/` and treats warnings as errors. On a system that is not Windows, clippy also runs for the Windows target if rustup has it, because some code compiles only on Windows.
"""

import os
import shutil
import subprocess
import sys

PROJECT_ROOT = os.path.realpath(os.path.dirname(__file__))


def _python_targets() -> list[str]:
    rel: list[str] = [
        d
        for d in ("src", "tests", "scripts")
        if os.path.isdir(os.path.join(PROJECT_ROOT, d))
    ]
    rel += [f for f in os.listdir(PROJECT_ROOT) if f.endswith(".py")]
    return [os.path.join(PROJECT_ROOT, r) for r in rel]


def _venv_python() -> str | None:
    """The project venv's interpreter, so pyright (run via uvx) resolves the deps and the editable package.

    Honors UV_PROJECT_ENVIRONMENT, falling back to the in-repo `.venv`; handles both the POSIX and the Windows layout.
    """
    venv_dir = os.environ.get(
        "UV_PROJECT_ENVIRONMENT", os.path.join(PROJECT_ROOT, ".venv")
    )
    for rel in (("bin", "python"), ("Scripts", "python.exe")):
        path = os.path.join(venv_dir, *rel)
        if os.path.exists(path):
            return path
    return None


def type_check_python() -> bool | None:
    """Run pyright. Returns True/False on pass/fail, or None if skipped."""
    targets = _python_targets()
    if not targets:
        print("No Python sources yet; nothing to type-check.")
        return None
    cmd = ["uvx", "pyright"]
    if venv_python := _venv_python():
        cmd += ["--pythonpath", venv_python]
    cmd += targets
    return subprocess.run(cmd, stdout=sys.stdout, stderr=sys.stderr).returncode == 0


WINDOWS_TARGET = "x86_64-pc-windows-msvc"


def _clippy(manifest: str, target: str | None = None) -> bool:
    cmd = ["cargo", "clippy", "--all-targets", "--manifest-path", manifest]
    if target is not None:
        cmd += ["--target", target]
    cmd += ["--", "-D", "warnings"]
    print("+", " ".join(cmd), flush=True)
    return subprocess.run(cmd, stdout=sys.stdout, stderr=sys.stderr).returncode == 0


def _target_installed(target: str) -> bool:
    """Whether rustup has the standard library for `target` in the toolchain that `native/rust-toolchain.toml` pins."""
    if shutil.which("rustup") is None:
        return False
    result = subprocess.run(
        ["rustup", "target", "list", "--installed"],
        cwd=os.path.join(PROJECT_ROOT, "native"),
        capture_output=True,
        text=True,
    )
    return result.returncode == 0 and target in result.stdout.split()


def lint_rust() -> bool | None:
    """Run clippy over `native/`, and also for the Windows target off Windows. Returns True/False on pass/fail, or None if skipped."""
    manifest = os.path.join(PROJECT_ROOT, "native", "Cargo.toml")
    if not os.path.exists(manifest):
        return None
    if shutil.which("cargo") is None:
        print("cargo not found; skipping clippy.")
        return None
    ok = _clippy(manifest)
    if sys.platform != "win32":
        if _target_installed(WINDOWS_TARGET):
            ok = _clippy(manifest, WINDOWS_TARGET) and ok
        else:
            print(
                f"The Windows target is not installed; skipping clippy for it. To add it: rustup target add {WINDOWS_TARGET}"
            )
    return ok


def main() -> None:
    results = [type_check_python(), lint_rust()]
    sys.exit(0 if all(ok in (True, None) for ok in results) else 1)


if __name__ == "__main__":
    main()
