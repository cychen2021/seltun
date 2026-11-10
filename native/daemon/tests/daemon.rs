//! Tests of the two parts that use the network, with no adapter. The first part is the client of the upstream, tested against stub SOCKS5 and HTTP proxies. The second part is the control channel, tested against a real `Daemon`.

use std::net::SocketAddr;
use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::Notify;

use seltund::config::{Cli, Protocol, Upstream};
use seltund::control;
use seltund::daemon::Daemon;
use seltund::socks::{self, UdpAssociation};

fn upstream(address: SocketAddr, protocol: Protocol) -> Upstream {
    Upstream {
        address,
        protocol,
        udp: true,
    }
}

/// A SOCKS5 proxy. It records the requested target, then returns the data that it receives in upper case. For `UDP ASSOCIATE`, it gives the UDP relay at `relay`.
async fn stub_socks5(relay: Option<SocketAddr>) -> (SocketAddr, tokio::task::JoinHandle<Vec<u8>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (mut s, _) = listener.accept().await.unwrap();
        let mut hello = [0u8; 3];
        s.read_exact(&mut hello).await.unwrap();
        assert_eq!(hello, [5, 1, 0]);
        s.write_all(&[5, 0]).await.unwrap();
        let mut head = [0u8; 4];
        s.read_exact(&mut head).await.unwrap();
        let len = if head[3] == 1 { 4 + 2 } else { 16 + 2 };
        let mut rest = vec![0u8; len];
        s.read_exact(&mut rest).await.unwrap();
        let mut request = head.to_vec();
        request.extend(rest);
        match relay {
            Some(relay) => {
                let SocketAddr::V4(relay) = relay else {
                    unreachable!()
                };
                let mut reply = vec![5, 0, 0, 1];
                reply.extend_from_slice(&relay.ip().octets());
                reply.extend_from_slice(&relay.port().to_be_bytes());
                s.write_all(&reply).await.unwrap();
                // Keep the control connection open until the client closes it.
                let mut sink = Vec::new();
                let _ = s.read_to_end(&mut sink).await;
            },
            None => {
                s.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]).await.unwrap();
                let mut data = [0u8; 5];
                s.read_exact(&mut data).await.unwrap();
                s.write_all(&data.to_ascii_uppercase()).await.unwrap();
            },
        }
        request
    });
    (address, task)
}

#[tokio::test]
async fn socks5_connect_names_the_target_and_tunnels() {
    let (address, request) = stub_socks5(None).await;
    let target: SocketAddr = "93.184.216.34:443".parse().unwrap();
    let mut stream = socks::connect(&upstream(address, Protocol::Socks5), target)
        .await
        .unwrap();
    stream.write_all(b"hello").await.unwrap();
    let mut answer = [0u8; 5];
    stream.read_exact(&mut answer).await.unwrap();
    assert_eq!(&answer, b"HELLO");
    assert_eq!(
        request.await.unwrap(),
        [5, 1, 0, 1, 93, 184, 216, 34, 0x01, 0xbb]
    );
}

#[tokio::test]
async fn socks5_refusal_is_an_error() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut s, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 3];
        s.read_exact(&mut buf).await.unwrap();
        s.write_all(&[5, 0]).await.unwrap();
        let mut request = [0u8; 10];
        s.read_exact(&mut request).await.unwrap();
        s.write_all(&[5, 5, 0, 1, 0, 0, 0, 0, 0, 0]).await.unwrap();
    });
    let result = socks::connect(
        &upstream(address, Protocol::Socks5),
        "1.2.3.4:80".parse().unwrap(),
    )
    .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn http_connect_leaves_the_tunnel_untouched() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let seen = tokio::spawn(async move {
        let (s, _) = listener.accept().await.unwrap();
        let mut s = BufReader::new(s);
        let mut first = String::new();
        s.read_line(&mut first).await.unwrap();
        let mut blank = String::new();
        while blank != "\r\n" {
            blank.clear();
            s.read_line(&mut blank).await.unwrap();
        }
        // The first tunneled bytes come after the 200 in the same segment.
        s.get_mut()
            .write_all(b"HTTP/1.1 200 Connection established\r\n\r\nearly")
            .await
            .unwrap();
        first
    });
    let mut stream = socks::connect(
        &upstream(address, Protocol::Http),
        "1.2.3.4:8080".parse().unwrap(),
    )
    .await
    .unwrap();
    let mut early = [0u8; 5];
    stream.read_exact(&mut early).await.unwrap();
    assert_eq!(&early, b"early");
    assert_eq!(seen.await.unwrap(), "CONNECT 1.2.3.4:8080 HTTP/1.1\r\n");
}

#[tokio::test]
async fn udp_association_wraps_and_unwraps_datagrams() {
    let relay = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let (address, _control) = stub_socks5(Some(relay.local_addr().unwrap())).await;
    let association = UdpAssociation::open(&upstream(address, Protocol::Socks5))
        .await
        .unwrap();
    let target: SocketAddr = "8.8.8.8:53".parse().unwrap();
    association.send_to(target, b"query").await.unwrap();

    let mut buf = [0u8; 64];
    let (n, client) = relay.recv_from(&mut buf).await.unwrap();
    assert_eq!(
        &buf[..n],
        [&[0, 0, 0, 1, 8, 8, 8, 8, 0, 53][..], b"query"].concat()
    );

    relay
        .send_to(
            &[&[0, 0, 0, 1, 8, 8, 8, 8, 0, 53][..], b"answer"].concat(),
            client,
        )
        .await
        .unwrap();
    let mut out = [0u8; 64];
    let (source, n) = association.recv(&mut out).await.unwrap();
    assert_eq!((source, &out[..n]), (target, &b"answer"[..]));
}

async fn ask(address: SocketAddr, cmd: &str) -> serde_json::Value {
    let stream = TcpStream::connect(address).await.unwrap();
    let mut stream = BufReader::new(stream);
    stream
        .get_mut()
        .write_all(format!("{{\"cmd\":\"{cmd}\"}}\n").as_bytes())
        .await
        .unwrap();
    let mut line = String::new();
    stream.read_line(&mut line).await.unwrap();
    serde_json::from_str(&line).unwrap()
}

#[tokio::test]
async fn control_reload_follows_the_state_file() {
    let dir = std::env::temp_dir().join("seltun-control-test");
    std::fs::create_dir_all(&dir).unwrap();
    let (config, state) = (dir.join("config.toml"), dir.join("state.json"));
    std::fs::write(&config, "").unwrap();
    std::fs::write(&state, r#"{"enabled": false}"#).unwrap();

    let daemon = Arc::new(Daemon::new(Cli {
        config,
        state: state.clone(),
        log_file: None,
    }));
    let quit = Arc::new(Notify::new());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(control::serve(listener, daemon, quit.clone()));

    let reply = ask(address, "ping").await;
    assert_eq!(reply["ok"], true);
    assert_eq!(reply["status"]["enabled"], false);

    // Switch on. The daemon reads the saved switch. No process is selected, thus nothing starts. A test must never create an adapter, with or without administrator rights. The status gives the reason.
    std::fs::write(&state, r#"{"enabled": true}"#).unwrap();
    let reply = ask(address, "reload").await;
    assert_eq!(reply["status"]["enabled"], true);
    assert_eq!(reply["status"]["active"], false);
    assert!(
        reply["status"]["error"]
            .as_str()
            .unwrap()
            .contains("names is empty")
    );

    assert_eq!(ask(address, "bogus").await["ok"], false);
    ask(address, "shutdown").await;
    tokio::time::timeout(std::time::Duration::from_secs(1), quit.notified())
        .await
        .unwrap();
}
