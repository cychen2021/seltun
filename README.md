# seltun

seltun sends the traffic of selected Windows or macOS processes to a local proxy. A system-wide TUN adapter steers all traffic of the computer. seltun steers only the processes that you select.

When seltun is on, a daemon creates a TUN adapter (Wintun on Windows, utun on macOS). The daemon sends the traffic of the selected processes through the adapter to the proxy that you configure (SOCKS5 or HTTP). All other traffic keeps its usual path.

A CLI sets the switch to on or off. The switch keeps its state when the computer starts again.

The tool works on Windows and macOS. See [docs/daemon.md](docs/daemon.md) and [docs/macos.md](docs/macos.md) for more information.

## Install

Make sure that you have [uv](https://docs.astral.sh/uv/) and a Rust toolchain. The install compiles the daemon and downloads the pinned Wintun driver DLL.

```sh
uv run path/to/seltun/scripts/install.py
```

This command installs seltun with `uv tool install` and puts `seltun` on `PATH`. On Windows, the package contains `seltund.exe` and `wintun.dll`. If autostart is installed, the script stops the daemon before the install and starts it again after the install.

On macOS, the script also signs the daemon `seltund` with a self-signed certificate. The script creates the certificate in the login keychain on the first run. With a stable signature, macOS keeps the background item and the permissions of the daemon after each new build. See [docs/macos.md](docs/macos.md).

## Use

1. Run `seltun init`. It creates the config file if the file is missing. On Windows, it downloads `wintun.dll` if the file is missing. `seltun paths` shows the location of the config file.
2. In the config file, write the processes to steer in `[processes].names`. Use file names, for example `哔哩哔哩.exe`, or full paths.
3. On Windows, open an elevated terminal and run `seltun autostart enable` one time. After this step, the daemon starts at each login with administrator rights. On macOS, run the same command in a usual terminal: `sudo` asks for your password one time. The daemon then runs as root and starts with the computer.
4. Run `seltun daemon start` to start the daemon. No UAC prompt and no password prompt shows.
5. Run `seltun switch on` to steer the selected processes. Run `seltun switch off` to stop.

These commands are also available:

| Command | Result |
| --- | --- |
| `seltun status` | Shows the switch, the daemon, autostart, and the problem if there is one. |
| `seltun daemon run` | Runs the daemon in the foreground. On Windows, use an elevated terminal. On macOS, `sudo` asks for your password. |
| `seltun daemon stop` | Stops the daemon. The switch keeps its state. |
| `seltun daemon reload` | Makes the daemon read the config file and the switch again. |
| `seltun autostart disable` | Removes autostart. On Windows, use an elevated terminal. On macOS, `sudo` asks for your password, and the daemon stops. |
| `seltun --version` | Shows the version. |

The proxy sees all forwarded traffic as traffic of the process `seltund.exe` (`seltund` on macOS). The value of `[upstream].address` is the address of the proxy. The default value is 127.0.0.1:1080.

These files hold the data of seltun:

| What | Windows | macOS |
| --- | --- | --- |
| Config file | `%APPDATA%\seltun\config.toml` | `~/Library/Application Support/seltun/config.toml` |
| Switch state | `%LOCALAPPDATA%\seltun\state.json` | `~/Library/Application Support/seltun/state.json` |
| Daemon log | `%LOCALAPPDATA%\seltun\Logs\seltund.log` | `~/Library/Logs/seltun/seltund.log` |

The options `--config` and `--state` change the first two locations. The file [`config.example.toml`](config.example.toml) is the template with comments. The command `seltun init` copies it. seltun does not keep credentials, because a local proxy port does not use them. Thus there is no `secrets.example.toml` file.

## Build and test

Run `uv sync` to install the CLI in editable mode. This command does not build the daemon. Run `cargo build --release --manifest-path native/Cargo.toml` to build the daemon. Run it again after each change to the Rust code. On Windows, run `seltun init` one time to get `wintun.dll`.

These commands do the checks:

- `uv run pytest`
- `cargo test --manifest-path native/Cargo.toml`
- `uv run format.py`
- `uv run type_check.py`
