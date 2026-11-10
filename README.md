# seltun

Clash Verge's TUN mode, for chosen processes only. A daemon creates a Wintun adapter while seltun is on, steers the selected processes' traffic into it and forwards that to Clash Verge's local port, which proxies it. Everything else keeps its normal path. A CLI switches it on and off; the state survives restarts.

> **Private use only.** This is a personal tool and is not publicly available.

Windows only for now. Design: [docs/daemon.md](docs/daemon.md).

## Install

Needs [uv](https://docs.astral.sh/uv/) and a Rust toolchain (the daemon is compiled during the install; the build also downloads the pinned Wintun driver DLL):

```sh
uv tool install path/to/seltun
```

This puts `seltun` on `PATH`, with `seltund.exe` and `wintun.dll` inside the package.

## Usage

```sh
seltun init                    # write the example config, then edit it (`seltun paths` shows where)
seltun autostart enable        # once, from an ELEVATED terminal: start the daemon at every login, elevated
seltun start                   # start the daemon now (no UAC prompt once the task exists)
seltun on                      # steer the selected processes; `seltun off` stops; persisted across restarts
seltun status                  # the switch, the daemon, what it is doing
seltun stop                    # stop the daemon
seltun daemon                  # or: run it in the foreground (elevated terminal)
```

List the processes under `[processes].names` (file names such as `哔哩哔哩.exe`, or full paths). In Clash Verge, route what the daemon forwards with the rule printed by `seltun rule <proxy-or-group>` (`PROCESS-NAME,seltund.exe,<proxy>`): Clash sees all forwarded traffic as coming from `seltund.exe`. `[upstream].address` is Clash Verge's mixed port (7897 by default).

| What | Windows |
| --- | --- |
| Config | `%APPDATA%\seltun\config.toml` |
| On/off state | `%LOCALAPPDATA%\seltun\state.json` |
| Daemon log | `%LOCALAPPDATA%\seltun\Logs\seltund.log` |

`--config` and `--state` override the first two. The annotated template is [`src/seltun/config.example.toml`](src/seltun/config.example.toml) (`seltun init` copies it). There is no `secrets.example.toml`: seltun holds no credentials, since Clash's local mixed port takes none.

## Development

`uv sync` installs the CLI in editable mode without building the daemon; `seltun` then uses `native/target/release/`, so run `cargo build --release --manifest-path native/Cargo.toml` after changing the Rust code, and `seltun setup` once for `wintun.dll`. `uv run pytest`, `cargo test --manifest-path native/Cargo.toml`, `uv run format.py` and `uv run type_check.py` are the checks.
