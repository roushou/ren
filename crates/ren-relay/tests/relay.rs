//! The relay on real sockets: a client asks it for tunnels as curl, git,
//! and cargo do through `HTTPS_PROXY`, to an echo server on this host
//! that a policy lists by its address.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::disallowed_methods,
    reason = "integration tests are a shell: helpers fail loudly, and the test spawns the relay and the echo server its caller would"
)]

use base64::Engine as _;
use ren_relay::{Course, Passage, Refusal, Relay, RelayPolicy, Ticket, TunnelEnd, Unreachable};
use std::{net::SocketAddr, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{mpsc, oneshot},
};

const SECRET: &str = "0123456789abcdef0123";

/// An echo server on loopback: it sends back what it reads.
async fn echo() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let (mut read, mut write) = stream.split();
                let _ = tokio::io::copy(&mut read, &mut write).await;
            });
        }
    });
    address
}

/// A port on loopback nothing listens on.
async fn closed_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    listener.local_addr().unwrap().port()
}

struct Running {
    relay: std::sync::Arc<Relay<&'static str>>,
    address: SocketAddr,
    passages: mpsc::UnboundedReceiver<Passage<&'static str>>,
    _stop: oneshot::Sender<()>,
}

async fn relay() -> Running {
    let relay = std::sync::Arc::new(Relay::new());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (sender, passages) = mpsc::unbounded_channel();
    let (stop, stopped) = oneshot::channel::<()>();
    let serving = relay.clone();
    tokio::spawn(async move {
        serving
            .serve(listener, sender, async {
                let _ = stopped.await;
            })
            .await;
    });
    Running {
        relay,
        address,
        passages,
        _stop: stop,
    }
}

fn policy(destinations: &[String]) -> RelayPolicy {
    RelayPolicy::new(destinations.iter().map(|text| text.parse().unwrap()))
}

/// Sends `CONNECT target` (with the ticket, when given) and `early`, and
/// returns the connection and the relay's answer's head.
async fn ask(
    relay: SocketAddr,
    request_line: &str,
    secret: Option<&str>,
    early: &[u8],
) -> (TcpStream, String) {
    let mut stream = TcpStream::connect(relay).await.unwrap();
    let mut head = format!("{request_line}\r\n");
    if let Some(secret) = secret {
        let credential = base64::engine::general_purpose::STANDARD.encode(format!("run:{secret}"));
        head.push_str(&format!("Proxy-Authorization: Basic {credential}\r\n"));
    }
    head.push_str("\r\n");
    let mut bytes = head.into_bytes();
    bytes.extend_from_slice(early);
    stream.write_all(&bytes).await.unwrap();
    let mut answer = Vec::new();
    let mut byte = [0_u8; 1];
    while !answer.ends_with(b"\r\n\r\n") {
        assert_eq!(
            stream.read(&mut byte).await.unwrap(),
            1,
            "the relay answers"
        );
        answer.push(byte[0]);
    }
    (stream, String::from_utf8(answer).unwrap())
}

async fn next(passages: &mut mpsc::UnboundedReceiver<Passage<&'static str>>) -> Course {
    let passage = tokio::time::timeout(Duration::from_secs(10), passages.recv())
        .await
        .expect("a passage is reported")
        .unwrap();
    assert_eq!(*passage.holder(), "run-1");
    passage.course().clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn an_admitted_tunnel_passes_bytes_both_ways_and_reports_them() {
    let echo = echo().await;
    let mut running = relay().await;
    let _pass = running
        .relay
        .issue(
            Ticket::new(SECRET).unwrap(),
            "run-1",
            policy(&[echo.to_string()]),
        )
        .unwrap();

    // The client may send its first bytes with its head, as TLS does.
    let (mut tunnel, answer) = ask(
        running.address,
        &format!("CONNECT {echo} HTTP/1.1"),
        Some(SECRET),
        b"hello ",
    )
    .await;
    assert_eq!(answer, "HTTP/1.1 200 Connection Established\r\n\r\n");
    tunnel.write_all(b"world").await.unwrap();
    let mut echoed = [0_u8; 11];
    tunnel.read_exact(&mut echoed).await.unwrap();
    assert_eq!(&echoed, b"hello world");
    drop(tunnel);

    match next(&mut running.passages).await {
        Course::Tunnel {
            target,
            end:
                TunnelEnd::Closed {
                    through,
                    sent,
                    received,
                },
        } => {
            assert_eq!(target.to_string(), echo.to_string());
            assert_eq!(through.to_string(), echo.to_string());
            assert_eq!((sent, received), (11, 11));
        }
        other @ (Course::Tunnel { .. } | Course::NotATunnel { .. }) => panic!("{other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn tunnels_its_policy_does_not_list_are_refused_and_reported() {
    let echo = echo().await;
    let closed = closed_port().await;
    let mut running = relay().await;
    let _pass = running
        .relay
        .issue(
            Ticket::new(SECRET).unwrap(),
            "run-1",
            policy(&[
                // Listed by name: localhost resolves only to this host.
                format!("localhost:{}", echo.port()),
                format!("127.0.0.1:{closed}"),
            ]),
        )
        .unwrap();

    let (_, answer) = ask(
        running.address,
        "CONNECT example.com:443 HTTP/1.1",
        Some(SECRET),
        b"",
    )
    .await;
    assert!(answer.starts_with("HTTP/1.1 403 Forbidden\r\n"), "{answer}");
    assert!(answer.contains("X-Ren-Refusal: unlisted"));
    assert!(matches!(
        next(&mut running.passages).await,
        Course::Tunnel {
            end: TunnelEnd::Refused(Refusal::Unlisted),
            ..
        }
    ));

    let (_, answer) = ask(
        running.address,
        &format!("CONNECT localhost:{} HTTP/1.1", echo.port()),
        Some(SECRET),
        b"",
    )
    .await;
    assert!(
        answer.contains("X-Ren-Refusal: private_address"),
        "{answer}"
    );
    assert!(matches!(
        next(&mut running.passages).await,
        Course::Tunnel {
            end: TunnelEnd::Refused(Refusal::PrivateAddress),
            ..
        }
    ));

    let (_, answer) = ask(
        running.address,
        &format!("CONNECT 127.0.0.1:{closed} HTTP/1.1"),
        Some(SECRET),
        b"",
    )
    .await;
    assert!(
        answer.starts_with("HTTP/1.1 502 Bad Gateway\r\n"),
        "{answer}"
    );
    assert!(matches!(
        next(&mut running.passages).await,
        Course::Tunnel {
            end: TunnelEnd::Unreachable {
                why: Unreachable::Refused,
                ..
            },
            ..
        }
    ));

    let (_, answer) = ask(
        running.address,
        "GET http://example.com/ HTTP/1.1",
        Some(SECRET),
        b"",
    )
    .await;
    assert!(
        answer.starts_with("HTTP/1.1 405 Method Not Allowed\r\n"),
        "{answer}"
    );
    assert_eq!(
        next(&mut running.passages).await,
        Course::NotATunnel {
            method: "GET".into()
        }
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn without_a_held_ticket_the_relay_asks_for_one_and_reports_nothing() {
    let echo = echo().await;
    let mut running = relay().await;
    let pass = running
        .relay
        .issue(
            Ticket::new(SECRET).unwrap(),
            "run-1",
            policy(&[echo.to_string()]),
        )
        .unwrap();
    assert!(
        running
            .relay
            .issue(
                Ticket::new(SECRET).unwrap(),
                "run-2",
                RelayPolicy::default()
            )
            .is_err(),
        "a ticket names one holder"
    );
    let line = format!("CONNECT {echo} HTTP/1.1");

    let (_, answer) = ask(running.address, &line, None, b"").await;
    assert!(answer.starts_with("HTTP/1.1 407 "), "{answer}");
    assert!(answer.contains("Proxy-Authenticate: Basic realm=\"ren\""));
    let (_, answer) = ask(running.address, &line, Some("someone-elses-ticket"), b"").await;
    assert!(answer.starts_with("HTTP/1.1 407 "), "{answer}");

    // Revoked with its handle.
    drop(pass);
    let (_, answer) = ask(running.address, &line, Some(SECRET), b"").await;
    assert!(answer.starts_with("HTTP/1.1 407 "), "{answer}");
    assert!(
        running.passages.try_recv().is_err(),
        "no holder, no passage"
    );
}
