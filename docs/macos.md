# macOS

seltun uses the same design on macOS as on Windows (see [daemon.md](daemon.md)). This page tells the differences.

## Data path

| Step | Windows | macOS |
| --- | --- | --- |
| TUN adapter | Wintun adapter with the name `[tun].name` | utun interface. macOS gives the name (`utunN`), thus the daemon does not use `[tun].name`. The peer address is the next address after `[tun].address`. |
| Routes | `netsh`: `0.0.0.0/1` and `128.0.0.0/1` | `route`: the same two routes with `-interface utunN` |
| Owner of a socket | `GetExtendedTcpTable` and `GetExtendedUdpTable` | libproc. macOS has no table of all sockets with their owners. Thus the daemon finds the selected processes first (`proc_pidpath`). Then it reads the sockets of these processes only (`proc_pidfdinfo`). |
| Physical interface | The default route with the lowest metric | `PrimaryInterface` of `State:/Network/Global/IPv4` (`scutil`). The utun interface is not a network service, thus the routes of the daemon do not change the primary interface. |
| Pin a socket to the physical interface | `IP_UNICAST_IF` | `IP_BOUND_IF`. The scoped default route of the physical interface then carries the socket. |

**Scoped default route.** macOS gives no scoped default route to the primary interface. A socket that is pinned to the primary interface then uses the global routing table, where the half-default routes of the TUN are the best match. The lookup fails, because the interface of these routes is not the pinned interface. This applies to the direct sockets of the daemon and also to the sockets of other programs that pin to the interface, for example the DNS resolver of macOS and VPN clients. Thus the daemon adds `route add -ifscope <interface> default <router>` before it adds the half-default routes. Every two seconds, it makes sure that a pinned lookup still works. It adds the route again when the network changes (for example from home to an office) or when macOS removes the routes of the interface. When the daemon stops, it removes the route.
| DNS servers of the adapter | The daemon copies them (`mirror_dns`) | Not necessary. macOS resolves names with its own resolver configuration, not with the servers of an interface. |
| Test for Internet access | The daemon ends the test of Windows | Not necessary. |

The code is in `native/daemon/src/platform/macos.rs` and in `create_device` in `native/daemon/src/daemon.rs`.

**Names of processes.** The name in `[processes].names` is the file name of the executable, not the name of the app. An app often has helper processes, and a helper can make the connections. For example, the network connections of a Chromium browser come from a helper process. Use `ps -axo comm` to find the executable path of a process.

**What the upstream proxy sees.** The proxy sees all forwarded traffic as traffic of the process `seltund`.

## Root and autostart

The creation of the utun interface and the routes needs root. `seltun autostart enable` asks for the password of the user one time (with `sudo`). It installs these two files:

- `/Library/LaunchDaemons/io.github.cychen2021.seltund.plist`: a launchd daemon. launchd starts `seltund` as root when the computer starts (`RunAtLoad`). It starts the daemon again after a failure, but not after `seltun daemon stop` (`KeepAlive` with `SuccessfulExit` set to false). The daemon starts immediately after `seltun autostart enable`.
- `/etc/sudoers.d/seltund`: a sudoers rule. It lets the user run `launchctl kickstart` for this daemon only, with no password. Thus `seltun daemon start` shows no prompt. This is the same function as the Task Scheduler task on Windows.

`seltun autostart disable` stops the daemon and removes the two files. `seltun daemon run` runs the daemon in the foreground with `sudo`.

The daemon reads the config file and the state file of the user, and it writes its log to `~/Library/Logs/seltun/seltund.log`. `seltun autostart enable` creates the log file first. Thus the file stays the property of the user.

The launchd daemon runs the binary in the install directory of uv as root. A process of the user can change this binary. This is the same risk as with the Task Scheduler task on Windows.

## Code signature

An ad-hoc signature identifies a binary by its hash. Each build then looks like a new program to macOS. Background items (System Settings > General > Login Items & Extensions) and each permission that macOS keeps for a program belong to the old build.

Thus `scripts/install-mac.sh` signs the daemon with a stable self-signed certificate:

1. On the first run, the script creates the certificate `seltun Code Signing` in the login keychain. The environment variable `SELTUN_CERT` changes the name.
2. It runs `uv tool install` again. The build hook (`hatch_build.py`) signs the daemon with the certificate and the identifier `io.github.cychen2021.seltund`, if the certificate is in the keychain.
3. It signs the installed daemon again, to be sure, and shows the designated requirement.
4. If the launchd daemon is installed, the script restarts it with the new binary. The sudoers rule permits this command with no password.

`scripts/install-mac.sh --no-install` only signs the daemon that is installed.

Only after the first signed build: if System Settings shows an old background item for `seltund`, remove it. The new build has a new identity one time. Later builds keep it.
