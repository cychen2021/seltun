# seltun

seltun sends the traffic of selected processes to Clash Verge. The TUN mode of Clash Verge steers all traffic of the system. seltun steers only the processes that you select.

When seltun is on, a daemon creates a Wintun adapter. The daemon sends the traffic of the selected processes through the adapter to the local port of Clash Verge. Clash Verge then sends the traffic to a proxy. All other traffic keeps its usual path.

A CLI sets the switch to on or off. The switch keeps its state when the computer starts again.

> **For the owner only.** Do not give this tool to other persons.

The tool works on Windows only. See [docs/daemon.md](docs/daemon.md) for more information.

## Install

Make sure that you have [uv](https://docs.astral.sh/uv/) and a Rust toolchain. The install compiles the daemon and downloads the pinned Wintun driver DLL.

```sh
uv tool install path/to/seltun
```

This command puts `seltun` on `PATH`. The package contains `seltund.exe` and `wintun.dll`.

## Use

1. Run `seltun init`. It creates the config file if the file is missing. It downloads `wintun.dll` if the file is missing. `seltun paths` shows the location of the config file.
2. In the config file, write the processes to steer in `[processes].names`. Use file names, for example `哔哩哔哩.exe`, or full paths.
3. Open an elevated terminal and run `seltun autostart enable` one time. After this step, the daemon starts at each login with administrator rights.
4. Run `seltun daemon start` to start the daemon. No UAC prompt shows.
5. Run `seltun switch on` to steer the selected processes. Run `seltun switch off` to stop.

These commands are also available:

| Command | Result |
| --- | --- |
| `seltun status` | Shows the switch, the daemon, the startup task, and the problem if there is one. |
| `seltun daemon run` | Runs the daemon in the foreground. Use an elevated terminal. |
| `seltun daemon stop` | Stops the daemon. The switch keeps its state. |
| `seltun reload` | Makes the daemon read the config file and the switch again. |
| `seltun autostart disable` | Removes the startup task. Use an elevated terminal. |
| `seltun --version` | Shows the version. |

Clash sees all forwarded traffic as traffic of the process `seltund.exe`. The value of `[upstream].address` is the mixed port of Clash Verge. The default value is 7897.

These files hold the data of seltun:

| What | Windows |
| --- | --- |
| Config file | `%APPDATA%\seltun\config.toml` |
| Switch state | `%LOCALAPPDATA%\seltun\state.json` |
| Daemon log | `%LOCALAPPDATA%\seltun\Logs\seltund.log` |

The options `--config` and `--state` change the first two locations. The file [`config.example.toml`](config.example.toml) is the template with comments. The command `seltun init` copies it. seltun does not keep credentials, because the local mixed port of Clash does not use them. Thus there is no `secrets.example.toml` file.

## Build and test

Run `uv sync` to install the CLI in editable mode. This command does not build the daemon. Run `cargo build --release --manifest-path native/Cargo.toml` to build the daemon. Run it again after each change to the Rust code. Run `seltun init` one time to get `wintun.dll`.

These commands do the checks:

- `uv run pytest`
- `cargo test --manifest-path native/Cargo.toml`
- `uv run format.py`
- `uv run type_check.py`
