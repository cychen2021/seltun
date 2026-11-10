#!/usr/bin/env -S uv run --script

# /// script
# requires-python = ">=3.12"
# dependencies = [
#     "ruff",
# ]
# ///
"""Format all languages of seltun: Python (ruff), Markdown (markdownlint-cli2 --fix), and Rust (cargo fmt).

A step does not run if its sources or its tool are not available.
"""

import os
import shutil
import subprocess
import sys

PROJECT_ROOT = os.path.realpath(os.path.dirname(__file__))

# Markdown directories that hold lintable docs (the precise globs/ignores live in `.markdownlint-cli2.jsonc`).
MD_DIRS = ("docs", ".vscode")


def _run(cmd: list[str]) -> None:
    print("+", " ".join(cmd), flush=True)
    subprocess.run(cmd, check=True, stdout=sys.stdout, stderr=sys.stderr)


def _python_targets() -> list[str]:
    rel: list[str] = [
        d
        for d in ("src", "tests", "scripts")
        if os.path.isdir(os.path.join(PROJECT_ROOT, d))
    ]
    rel += [f for f in os.listdir(PROJECT_ROOT) if f.endswith(".py")]
    return [os.path.join(PROJECT_ROOT, r) for r in rel]


def _find(dirs: tuple[str, ...], exts: tuple[str, ...]) -> list[str]:
    found: list[str] = []
    for d in dirs:
        base = os.path.join(PROJECT_ROOT, d)
        if not os.path.isdir(base):
            continue
        for root, _, files in os.walk(base):
            found += [os.path.join(root, f) for f in files if f.endswith(exts)]
    return found


def format_python() -> None:
    targets = _python_targets()
    if targets:
        _run(["uvx", "ruff", "format", *targets])


def _markdownlint_cmd() -> list[str] | None:
    """Resolve how to invoke markdownlint-cli2: prefer the installed binary, then npx."""
    if shutil.which("markdownlint-cli2"):
        return ["markdownlint-cli2"]
    if npx := shutil.which("npx"):
        return [npx, "--yes", "markdownlint-cli2"]
    return None


def format_markdown() -> None:
    has_md = any(f.endswith(".md") for f in os.listdir(PROJECT_ROOT)) or _find(
        MD_DIRS, (".md",)
    )
    if not has_md:
        return
    cmd = _markdownlint_cmd()
    if cmd is None:
        print("markdownlint-cli2 not found (and no npx); skipping Markdown formatting.")
        return
    # `--fix` applies auto-fixes; file selection comes from `.markdownlint-cli2.jsonc`.
    _run([*cmd, "--fix"])


def format_rust() -> None:
    """`cargo fmt` over the `native/` workspace, so somebody working on the Python half needs no Rust toolchain."""
    if not os.path.exists(os.path.join(PROJECT_ROOT, "native", "Cargo.toml")):
        return
    if shutil.which("cargo") is None:
        print("cargo not found; skipping Rust formatting.")
        return
    _run(
        [
            "cargo",
            "fmt",
            "--all",
            "--manifest-path",
            os.path.join("native", "Cargo.toml"),
        ]
    )


def main() -> None:
    os.chdir(PROJECT_ROOT)
    format_python()
    format_markdown()
    format_rust()


if __name__ == "__main__":
    main()
