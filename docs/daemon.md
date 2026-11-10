# The daemon (`seltund`)

## What Clash Verge's TUN does, and what seltun changes

Clash Verge runs mihomo with a `tun:` section (this machine: `stack: gvisor`, `auto-route: true`, `auto-detect-interface: true`, `dns-hijack: any:53`). In that mode mihomo:

1. creates a **Wintun** adapter;
2. **auto-routes**: adds half-default routes (`0.0.0.0/1`, `128.0.0.0/1`) onto the adapter, so every destination without a more specific route enters it;
3. terminates the flows in a **userspace TCP/IP stack** (gVisor);
4. matches each flow against the rules (`PROCESS-NAME` rules look up the process that owns the source socket), and sends it out the proxy or `DIRECT`;
5. **auto-detects the interface**: its own outbound sockets are pinned to the real NIC, so they do not loop back into the adapter.

seltun keeps that shape (Wintun, half-default routes, userspace stack, process lookup, pinned outbound sockets) and replaces the rule engine with one question: *is the owning process in `[processes].names`?* Yes → the flow is forwarded to Clash's mixed port (`[upstream]`), which proxies it. No → it goes out the real interface unchanged. The TUN adapter and routes exist only while seltun is **on**.

An alternative considered first was WinDivert (steer only the selected processes' packets, leave everything else untouched). It was dropped because Windows Defender quarantines the WinDivert driver as potentially unwanted software, and weakening antivirus is not something this tool should ask for. The cost of the Wintun design: while on, all IPv4 TCP/UDP of the machine passes through the daemon (non-selected flows are just relayed to their destination).

## Data path

`native/daemon/src/forward.rs`. Windows routes a flow into the adapter, so its source address is the adapter's own (`[tun].address`). `ipstack` terminates it and yields a TCP stream or UDP datagram stream with the original source and destination.

- **Owner lookup** (`platform/windows.rs`): `GetExtendedTcpTable` / `GetExtendedUdpTable` find the pid owning the source socket (TCP: matched on the full 4-tuple; UDP: on the local port), `QueryFullProcessImageNameW` gives its path, and `Matcher` compares it with the configured names or paths. An owner that cannot be found (system processes, a socket that already closed) counts as not selected.
- **Selected, TCP**: a SOCKS5 `CONNECT` (or HTTP `CONNECT`) to `[upstream].address` for the destination IP. Clash only ever sees IPs: the process resolved the name itself, so domain rules in Clash do not apply to this traffic; use `PROCESS-NAME` and IP rules.
- **Selected, UDP**: with `[upstream].udp = true`, a SOCKS5 `UDP ASSOCIATE`. Otherwise DNS (port 53) goes direct and everything else is dropped, so applications (QUIC, say) fall back to TCP.
- **Not selected**: a socket pinned to the real interface with `IP_UNICAST_IF` connects to the destination. The real interface is the best default route that is not the adapter (`GetIpForwardTable2`), re-evaluated every two seconds so Wi-Fi/Ethernet changes are followed.
- **ICMP echo** is answered locally (a real echo cannot be forwarded from userspace), as Clash's TUN does, so `ping` succeeds while on.

### What Clash sees

Every connection to Clash is made by `seltund.exe`, so Clash identifies all forwarded traffic as the process `seltund.exe`. `seltun rule <proxy>` prints `PROCESS-NAME,seltund.exe,<proxy>` for the Clash profile. Do not rename the binary without changing that.

## Control channel and the persisted switch

The daemon runs whenever it is started (at login via autostart, or `seltun start`) and is "on" or "off". The switch is `state.json` (`{"enabled": bool}` in the data directory); the CLI writes it atomically and then sends `reload` to `[daemon].control` (loopback, JSON lines: `{"cmd": "ping|status|reload|shutdown"}`). `reload` re-reads the state and the config and reconciles: on and not running → create the adapter and routes and start forwarding; off and running → tear them down; settings changed → restart. A broken config or state is reported and leaves what is running alone. At startup the daemon reconciles once, so it comes back in the state the user left. Requests carry no data, so the channel cannot be used to feed the (elevated) daemon anything but "look at the user's files again".

The control port is also the single-instance guard: a second daemon fails to bind it. A changed `[daemon].control` needs a daemon restart.

## Elevation and autostart

Creating the adapter and its routes needs administrator rights. A Run-key entry cannot elevate silently, so autostart is a Task Scheduler **logon task** with the highest privileges (`autostart.py`): `seltun autostart enable` creates it once from an elevated terminal; afterwards `seltun start` (runs the task) and login start the daemon elevated with no prompt. Without the task, `seltun daemon` in an elevated terminal runs it in the foreground. An unelevated daemon still answers the control channel and reports why it cannot turn on.

`wintun.dll` must sit next to `seltund.exe` (`seltun setup`, or the wheel build). It is fetched from wintun.net and pinned by SHA-256 in `drivers.py`.

## Limitations

- IPv4 only: no IPv6 routes are added, so IPv6 traffic of selected processes is not steered.
- Only new connections after the switch is turned on are steered; existing ones continue as they were.
- Child processes are not followed; list them too.
- Domain-based Clash rules do not see this traffic (IP only).
- Do not run Clash Verge's own TUN at the same time: both add half-default routes.
