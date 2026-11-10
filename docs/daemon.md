# The daemon (`seltund`)

## Design

seltun uses the design of a system-wide TUN adapter, and it adds a filter for processes. The daemon does these steps when the switch is on:

1. It creates a **Wintun** adapter.
2. It adds two half-default routes: `0.0.0.0/1` and `128.0.0.0/1`. Each destination without a more specific route then enters the adapter.
3. It ends the flows in a **userspace TCP/IP stack** (`ipstack`).
4. It finds the process that owns the source socket of each flow. Then it asks one question: is the process in `[processes].names`?
5. It pins its own outbound sockets to the physical network interface. The sockets then do not go back into the adapter.

The answer to the question in step 4 decides the path of the flow:

- If yes, the daemon sends the flow to the upstream proxy (`[upstream]`). The proxy is a local SOCKS5 or HTTP proxy.
- If no, the flow leaves through the physical interface without a change.

The adapter and the routes are present only when seltun is on.

The first design used WinDivert. WinDivert can steer the packets of the selected processes and does not touch other traffic. Windows Defender puts the WinDivert driver in quarantine as potentially unwanted software. Thus seltun does not use WinDivert. A weaker antivirus setting is not an option for this tool.

The Wintun design has one disadvantage. When seltun is on, all IPv4 TCP and UDP traffic of the computer goes through the daemon. The daemon sends the flows of the other processes to their destinations without a change.

## Data path

The code is in `native/daemon/src/forward.rs`.

Windows sends a flow into the adapter. The source address of the flow is then the address of the adapter (`[tun].address`). The library `ipstack` ends the flow. It gives a TCP stream or a UDP datagram stream with the original source and destination.

**Owner lookup** (`platform/windows.rs`):

- `GetExtendedTcpTable` and `GetExtendedUdpTable` give the pid of the process that owns the source socket. For TCP, the lookup uses the full 4-tuple. For UDP, it uses the local port.
- `QueryFullProcessImageNameW` gives the path of the executable.
- `Matcher` compares the path with the names or paths in the config file.
- If the daemon does not find the owner of a flow, the flow counts as not selected. Examples are the flow of a system process and the flow of a closed socket.

**Selected process, TCP:** The daemon sends a SOCKS5 `CONNECT` or an HTTP `CONNECT` to `[upstream].address` for the destination IP. The upstream proxy receives only IP addresses, because the process resolved the name before. Thus rules of the proxy that match a domain do not apply to this traffic.

**Selected process, UDP:**

- If `[upstream].udp` is `true`, the daemon uses a SOCKS5 `UDP ASSOCIATE`.
- If `[upstream].udp` is `false`, DNS (port 53) goes direct. The daemon drops all other UDP. The application then changes to TCP. QUIC is an example.

**Other processes:** A socket that the daemon pins to the physical interface with `IP_UNICAST_IF` connects to the destination. The physical interface has the default route with the lowest metric. It must be up and connected, and it is not the adapter. `GetIpForwardTable2` gives the routes. The daemon looks for the physical interface again after two seconds. Thus it follows a change between Wi-Fi and Ethernet.

**Network icon:** Windows tests each adapter for Internet access. If the test passes on the adapter, Windows can use the adapter as the main connection. The taskbar then shows "LAN" and not Wi-Fi. The adapter has the interface metric 9999, but the metric does not prevent this, because the routes of the adapter are more specific than the default route. Thus the daemon ends the HTTP request of the test (`www.msftconnecttest.com`) when it arrives in the adapter. The test then fails on the adapter only, and Windows keeps the physical connection as the main connection. The test on the physical interface does not use the adapter.

**ICMP echo:** The daemon sends the ping replies. A userspace program cannot forward an ICMP echo. Thus each ping gets a reply when seltun is on.

### What the upstream proxy sees

The daemon makes each connection to the upstream proxy. Thus the proxy identifies all forwarded traffic as the process `seltund.exe`. A rule of the proxy that matches the process name `seltund.exe` matches this traffic. seltun does not change the configuration of the proxy. Do not change the name of the executable, because users can write rules for it.

## Control channel and switch state

The daemon runs after you start it. Start it at login with autostart, or with `seltun daemon start`. The daemon is on or off.

The switch state is in `state.json`. The file is in the data directory and has this form: `{"enabled": true}`. The CLI writes the file in one atomic step. Then the CLI sends `reload` to the address `[daemon].control`.

The control channel is a loopback TCP port. It uses JSON lines, for example `{"cmd": "reload"}`. The commands are `ping`, `status`, `reload` and `shutdown`.

The command `reload` makes the daemon read the state and the config file again. Then the daemon does one of these actions:

- If the switch is on and the data path is not up, the daemon creates the adapter and the routes. Then it starts to forward the flows.
- If the switch is off and the data path is up, the daemon removes the adapter and the routes.
- If the settings changed, the daemon starts the data path again.

If the config file or the state file has an error, the daemon reports the error. It does not change the data path that is up. At startup, the daemon does one `reload`. Thus it comes back in the state that you selected.

A request has no data. Thus the control channel can give the elevated daemon only one instruction: read your files again.

The control port also prevents a second daemon. A second daemon cannot bind the port. A new value of `[daemon].control` applies after the daemon starts again.

## Elevation and autostart

The creation of the adapter and the routes needs administrator rights. A Run-key entry cannot use these rights without a UAC prompt. Thus autostart is a Task Scheduler **logon task** with the highest privileges (`autostart.py`).

1. Open an elevated terminal. Run `seltun autostart enable` one time.
2. After this step, `seltun daemon start` runs the task. The task also starts at each login. The daemon then runs with administrator rights, and no prompt shows.

Without the task, run `seltun daemon run` in an elevated terminal. The daemon then runs in the foreground.

`seltund.exe` is a program of the Windows subsystem. It has no console window. Thus Task Scheduler does not open a window, and no user can stop the daemon when a window closes. `seltun daemon run` is different: the daemon attaches to the terminal that started it, and it writes its log there. Ctrl+C stops it. If the output goes to a file or a pipe, the daemon keeps that output. A run with `--log-file` never uses a terminal.

A daemon that is not elevated answers on the control channel. It reports the reason why it cannot turn on.

`wintun.dll` must be in the same directory as `seltund.exe`. `seltun init` or the wheel build puts it there. The file comes from wintun.net. `drivers.py` has the SHA-256 value that the file must have.

## Limits

- seltun supports IPv4 only. It adds no IPv6 routes. Thus the IPv6 traffic of the selected processes does not go to the upstream proxy.
- seltun steers only new connections. A connection that was open before you set the switch to on is not changed.
- seltun does not follow child processes. Add them to the list.
- The upstream proxy sees only IP addresses. Thus its rules that match a domain do not apply.
- Do not use another system-wide TUN program at the same time. Both add half-default routes.
