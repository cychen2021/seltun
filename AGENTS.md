# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

It holds only what is worth knowing before any task in this repo. Everything worth knowing *when working on a particular part* lives in `docs/`, indexed by [docs/index.md](docs/index.md). Read the relevant page before changing that part; do not re-derive it.

> **Private use only.** This project is for the owner's personal use. Do not push it to a public remote, publish it as a package, or otherwise distribute it.

## Project purpose

seltun is Clash Verge's TUN mode narrowed to chosen processes. Clash Verge's TUN (mihomo + Wintun) captures the whole system; seltun creates a Wintun adapter that only the **selected processes** are steered into, and forwards what arrives there to a destination — Clash Verge's mixed port — which proxies it. Everything else keeps using the normal network path.

It is a CLI plus a daemon, like a sibling CLI project:

- `seltun` (Python, Typer): config, the persisted on/off switch (`state.json`), starting the daemon, autostart.
- `seltund` (Rust, `native/daemon/`): the daemon. Creates the TUN adapter and routes while "on", decides each flow's process, forwards selected flows to Clash and the rest out the real interface. Controlled over a loopback JSON-lines port.

The daemon's executable name is deliberate: every connection it makes to Clash comes from the process `seltund.exe`, which is what Clash's `PROCESS-NAME` rules match. See [docs/daemon.md](docs/daemon.md) for the data path and why it is built this way (it follows Clash Verge's TUN, not a WinDivert redirect: Defender quarantines WinDivert).

Windows only for now (Wintun, IP Helper, Task Scheduler); the pure Rust modules and the Python layer build and test anywhere.

## Toolchain and commands

This project uses **uv** (not pip/poetry) and targets **Python 3.14**; the daemon in `native/` is a Cargo workspace (edition 2024, toolchain pinned in `native/rust-toolchain.toml`). Validate with the project interpreter (`uv run python`, `uv run pytest`), not a system `python` — ruff formats to 3.14-only syntax such as PEP 758's unparenthesized `except A, B:`; do not "fix" it by adding parentheses.

- Install / sync deps: `uv sync` (editable; does not build the daemon — `cargo build --release --manifest-path native/Cargo.toml` does, and `seltun` falls back to `native/target/release/`; `seltun init` fetches `wintun.dll` next to it)
- Install as a tool: `uv tool install --force .` (the `hatch_build.py` hook compiles the daemon and downloads the pinned Wintun into `seltun/bin/`)
- Run the CLI: `uv run seltun --help`
- Tests: `uv run pytest`; `cargo test --manifest-path native/Cargo.toml`. Tests never create the adapter, elevated or not.
- Format: `uv run format.py` (ruff, markdownlint-cli2, `cargo fmt`; each step skips when its tooling is absent).
- Type-check: `uv run type_check.py` (`uvx pyright`, then `cargo clippy -D warnings`).

**After any significant code change, run both `format.py` and `type_check.py`** before handing back or committing — there is no CI, so these scripts are the authoritative gate.

Running the daemon for real needs an **elevated** terminal (Wintun adapter, routes): `seltun daemon run` in the foreground, or `seltun autostart enable` once (an elevated Task Scheduler task) and then `seltun daemon start` from any terminal.

## Repo layout

```text
.vscode/                 # settings.json + commit-general.instructions.md
docs/                    # per-topic reference (see docs/index.md)
src/seltun/
  config.py              # config.toml loader (mirrors native/daemon/src/config.rs)
  state.py               # state.json: the persisted on/off switch
  paths.py               # per-user config/data/log locations
  control.py             # client of the daemon's control channel
  daemon.py              # find and launch seltund
  autostart.py           # elevated Task Scheduler logon task
  drivers.py             # pinned download of wintun.dll
  cli/                   # Typer commands (thin layer)
  support/               # console output (loguru), error types
native/daemon/src/       # seltund (Rust): config, state, matcher, socks (Clash client), forward (data path), daemon (reconcile), control, platform (Windows glue)
tests/                   # pytest
config.example.toml      # annotated config template; `seltun init` copies it (the wheel bundles it)
hatch_build.py           # build hook: compile the daemon, fetch Wintun, bundle the template, into the wheel
format.py / type_check.py
```

## Conventions

- **Run the gate after significant changes**: `format.py` then `type_check.py`.
- **Imports: at most two leading dots.** Use relative imports up to `..` only; anything that would need `...` uses the absolute path instead.
- **Don't use hard line wraps** except in doc comments in source code or where explicitly required — one line per paragraph everywhere else, including EVERY markdown file, TOML comments, and commit-message bullets. `MD013` is switched off in `.markdownlint-cli2.jsonc` for this reason.
- **Config is read twice** (Python `config.py`, Rust `config.rs`) with the same defaults and the same strictness (unknown keys rejected); change both together.
- **Commit messages** follow `.vscode/commit-general.instructions.md` (English; headline a capitalized verb < 10 words, no trailing period; blank line before the body; backtick `code_item` and `path/filename`; prefer "moved"/"renamed" over delete+add; avoid vague verbs like "refactor"/"improve" unless nothing fits; one line for minor changes).
- **Docs live in `docs/`, named `snake_case.md`**, and every one is listed in [docs/index.md](docs/index.md). A new page means a new index row.
