# CLAUDE.md

This file gives guidance to Claude Code (claude.ai/code) for work in this repository.

The file has only the information that applies to each task. The pages in `docs/` have the information for one part of the project. [docs/index.md](docs/index.md) lists them. Read the page for the part that you change. Do not find the information again.

> **For the owner only.** Do not push this project to a public remote. Do not publish it as a package. Do not give it to other persons.

## Project purpose

seltun is a TUN adapter for selected processes only. A system-wide TUN adapter captures the whole computer. seltun creates a Wintun adapter and steers only the **selected processes** into it. Then it forwards the flows to a local SOCKS5 or HTTP proxy (the upstream). All other traffic keeps its usual path.

seltun has a CLI and a daemon, as a sibling CLI project has:

- `seltun` (Python, Typer) has the config, the persistent on/off switch (`state.json`), the start of the daemon and autostart.
- `seltund` (Rust, `native/daemon/`) is the daemon. When the switch is on, it creates the TUN adapter and the routes. It finds the process of each flow. It forwards the flows of the selected processes to the upstream proxy. It sends the other flows out through the physical interface. A loopback JSON-lines port controls the daemon.

The name of the daemon executable is intentional. Each connection from the daemon to the upstream proxy comes from the process `seltund.exe`. A rule of the proxy that matches the process name can match this name. See [docs/daemon.md](docs/daemon.md) for the data path and the reasons for the design. The design uses a Wintun adapter with half-default routes. It does not use a WinDivert redirect, because Defender puts WinDivert in quarantine.

The project works on Windows only (Wintun, IP Helper, Task Scheduler). The pure Rust modules and the Python layer build and test on each system.

## Toolchain and commands

The project uses **uv** (not pip or poetry) and **Python 3.14**. The daemon in `native/` is a Cargo workspace (edition 2024). `native/rust-toolchain.toml` pins the toolchain. Use the project interpreter (`uv run python`, `uv run pytest`), not a system `python`. Ruff formats the code to syntax that only Python 3.14 supports, for example the PEP 758 form `except A, B:` without parentheses. Do not add parentheses.

- Install the dependencies: `uv sync`. This command installs the CLI in editable mode and does not build the daemon. `cargo build --release --manifest-path native/Cargo.toml` builds the daemon. `seltun` uses `native/target/release/` as a fallback. `seltun init` puts `wintun.dll` next to the daemon.
- Install as a tool: `uv tool install --force .`. The `hatch_build.py` hook compiles the daemon. It also downloads the pinned Wintun into `seltun/bin/`.
- Run the CLI: `uv run seltun --help`.
- Run the tests: `uv run pytest` and `cargo test --manifest-path native/Cargo.toml`. The tests never create the adapter, also in an elevated terminal.
- Format: `uv run format.py`. It runs ruff, markdownlint-cli2 and `cargo fmt`. A step does not run if its tool is not available.
- Type-check: `uv run type_check.py`. It runs `uvx pyright` and then `cargo clippy -D warnings`.

**After a large code change, run `format.py` and then `type_check.py`.** Do this before you commit. There is no CI. These scripts are the authoritative gate.

A real run of the daemon needs an **elevated** terminal, because of the Wintun adapter and the routes. Run `seltun daemon run` in the foreground. Or run `seltun autostart enable` one time. After this step, `seltun daemon start` works from each terminal.

## Repo layout

```text
.vscode/                 # settings.json and commit-general.instructions.md
docs/                    # one reference page for each topic (see docs/index.md)
src/seltun/
  config.py              # config.toml loader (same rules as native/daemon/src/config.rs)
  state.py               # state.json: the persistent on/off switch
  paths.py               # per-user locations of config, data and log
  control.py             # client of the control channel of the daemon
  daemon.py              # finds and starts seltund
  autostart.py           # elevated Task Scheduler logon task
  drivers.py             # pinned download of wintun.dll
  cli/                   # Typer commands (thin layer)
  support/               # console output (loguru) and error types
native/daemon/src/       # seltund (Rust): config, state, matcher, socks (client of the upstream proxy), forward (data path), daemon (reconcile), control, platform (Windows code)
tests/                   # pytest
config.example.toml      # template with comments; `seltun init` copies it (the wheel contains it)
hatch_build.py           # build hook: compiles the daemon, downloads Wintun, adds the template to the wheel
format.py, type_check.py
```

## Conventions

- **Standard for text:** Write all documentation and all help and error messages in ASD-STE100 Simplified Technical English, Issue 9. The standard is in `C:\Users\cychen\OneDrive\Documents\ASD-STE100_ISSUE9.pdf`.
- **No external proxy software:** Do not name a proxy program in the docs, the help text, the messages or the comments. seltun forwards to any SOCKS5 or HTTP proxy at the address in `[upstream].address`.
- **Gate:** Run `format.py` and then `type_check.py` after a large change.
- **Imports:** Use at most two leading dots. If an import needs three dots, use the absolute path.
- **Line breaks:** Do not use hard line wraps, except in doc comments in source code or where a tool needs them. Write one line for each paragraph in each markdown file, each TOML comment and each commit-message bullet. `.markdownlint-cli2.jsonc` switches off `MD013` for this reason.
- **Version:** The version is in two places. `version` in `pyproject.toml` is the value that `seltun --version` shows (`src/seltun/_version.py` reads it). `[workspace.package].version` in `native/Cargo.toml` is the value that `seltund --version` shows. Change both together.
- **Config:** Python (`config.py`) and Rust (`config.rs`) both read the config. They use the same defaults and the same strict rules, and both reject unknown keys. Change both together.
- **Commit messages:** Follow `.vscode/commit-general.instructions.md`. Write in English. Start the headline with a verb that has a capital letter. Use less than 10 words and no period at the end. Put a blank line before the body. Put backticks around `code_item` and `path/filename`. Use "moved" or "renamed" instead of delete and add. Do not use vague verbs such as "refactor" or "improve" if another verb is better. Use one line for a minor change.
- **Docs:** Put docs in `docs/` and give them `snake_case.md` names. List each page in [docs/index.md](docs/index.md). A new page needs a new row.
