# The daemon (`seltund`)

## The TUN mode of Clash Verge and the changes in seltun

Clash Verge runs mihomo with a `tun:` section. On the computer of the owner, the section has these values: `stack: gvisor`, `auto-route: true`, `auto-detect-interface: true` and `dns-hijack: any:53`. In this mode, mihomo does these steps:

1. It creates a **Wintun** adapter.
2. It adds **auto-route** routes. These are the two half-default routes `0.0.0.0/1` and `128.0.0.0/1`. Each destination without a more specific route then enters the adapter.
3. It ends the flows in a **userspace TCP/IP stack** (gVisor).
4. It compares each flow with the rules. A `PROCESS-NAME` rule finds the process that owns the source socket.
5. It sends the flow through a proxy or `DIRECT`.
6. It pins its own outbound sockets to the physical network interface (**auto-detect-interface**). The sockets then do not go back into the adapter.

seltun keeps this design: Wintun, half-default routes, a userspace stack, a process lookup and pinned outbound sockets. seltun replaces the rule engine with one question: is the owning process in `[processes].names`?

- If yes, seltun sends the flow to the mixed port of Clash (`[upstream]`). Clash then sends it through a proxy.
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

**Selected process, TCP:** The daemon sends a SOCKS5 `CONNECT` or an HTTP `CONNECT` to `[upstream].address` for the destination IP. Clash receives only IP addresses, because the process resolved the name before. Thus the domain rules of Clash do not apply to this traffic. Use `PROCESS-NAME` rules and IP rules.

**Selected process, UDP:**

- If `[upstream].udp` is `true`, the daemon uses a SOCKS5 `UDP ASSOCIATE`.
- If `[upstream].udp` is `false`, DNS (port 53) goes direct. The daemon drops all other UDP. The application then changes to TCP. QUIC is an example.

**Other processes:** A socket that the daemon pins to the physical interface with `IP_UNICAST_IF` connects to the destination. The physical interface has the default route with the lowest metric. It must be up and connected, and it is not the adapter. `GetIpForwardTable2` gives the routes. The daemon looks for the physical interface again after two seconds. Thus it follows a change between Wi-Fi and Ethernet.

**ICMP echo:** The daemon sends the ping replies. A userspace program cannot forward an ICMP echo. The TUN mode of Clash does the same. Thus each ping gets a reply when seltun is on.

### What Clash sees

The daemon makes each connection to Clash. Thus Clash identifies all forwarded traffic as the process `seltund.exe`. A rule `PROCESS-NAME,seltund.exe` in the Clash profile matches this traffic. seltun does not change the Clash configuration. The name `seltund.exe` is part of the rules that users write for Clash. Do not change the name.

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

A daemon that is not elevated answers on the control channel. It reports the reason why it cannot turn on.

`wintun.dll` must be in the same directory as `seltund.exe`. `seltun init` or the wheel build puts it there. The file comes from wintun.net. `drivers.py` has the SHA-256 value that the file must have.

## Limits

- seltun supports IPv4 only. It adds no IPv6 routes. Thus the IPv6 traffic of the selected processes does not go to Clash.
- seltun steers only new connections. A connection that was open before you set the switch to on is not changed.
- seltun does not follow child processes. Add them to the list.
- The domain rules of Clash do not see this traffic. They see only the IP addresses.
- Do not use the TUN mode of Clash Verge at the same time. Both add half-default routes.
