use super::*;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::{ServerConfig, ServerConnection};
use std::net::TcpListener;
use tungstenite::handshake::server::{Request as Asked, Response as Answered};

/// What a test server saw of a socket: the handshake's headers, then each message, as text.
type Seen = Arc<Mutex<Vec<String>>>;

/// How a test server answers a socket after its handshake.
#[derive(Clone, Copy)]
enum Behaviour {
    /// Every message sent back; a close answered.
    Echo,
    /// Closes with 4001 at once.
    Closes,
    /// Ends the connection with no close frame.
    Drops,
    /// Sends "0" to "199" at once, then echoes.
    Counts,
}

/// The tests' certificate authority, and a server certificate for `localhost` it signed.
struct Authority {
    root: CertificateDer<'static>,
    server: ServerConfig,
}

fn authority() -> Authority {
    let ca_key = rcgen::KeyPair::generate().unwrap();
    let mut ca = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
    ca.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    ca.distinguished_name
        .push(rcgen::DnType::CommonName, "OxyOS tests' authority");
    ca.key_usages = vec![
        rcgen::KeyUsagePurpose::KeyCertSign,
        rcgen::KeyUsagePurpose::CrlSign,
    ];
    let root = ca.self_signed(&ca_key).unwrap();
    let issuer = rcgen::Issuer::new(ca, ca_key);
    let key = rcgen::KeyPair::generate().unwrap();
    let mut leaf = rcgen::CertificateParams::new(vec!["localhost".to_string()]).unwrap();
    leaf.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth];
    let certificate = leaf.signed_by(&key, &issuer).unwrap();
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let server = ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![certificate.der().clone()],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der())),
        )
        .unwrap();
    Authority {
        root: root.der().clone(),
        server,
    }
}

/// Options trusting the system's certificates and `root`, online as `online` says, no proxy.
fn options(root: Option<CertificateDer<'static>>, online: bool) -> Options {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let verifier =
        rustls_platform_verifier::Verifier::new_with_extra_roots(root, provider.clone()).unwrap();
    Options {
        tls: tls_config(provider, verifier).unwrap(),
        online: Arc::new(move || online),
        proxy: None,
    }
}

/// A WebSocket server on a port of its own (TLS with `tls`), answering as `behaviour` says.
fn server(behaviour: Behaviour, tls: Option<Arc<ServerConfig>>) -> (u16, Seen) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen: Seen = Arc::default();
    let record = seen.clone();
    std::thread::spawn(move || {
        for tcp in listener.incoming().flatten() {
            let (record, tls) = (record.clone(), tls.clone());
            std::thread::spawn(move || match tls {
                Some(tls) => {
                    let connection = ServerConnection::new(tls).unwrap();
                    serve(StreamOwned::new(connection, tcp), behaviour, record);
                }
                None => serve(tcp, behaviour, record),
            });
        }
    });
    (port, seen)
}

fn serve<S: Read + Write>(stream: S, behaviour: Behaviour, seen: Seen) {
    let record = seen.clone();
    let answer = move |asked: &Asked, mut answered: Answered| {
        let mut headers: Vec<String> = asked
            .headers()
            .iter()
            .map(|(name, value)| format!("{name}: {}", value.to_str().unwrap()))
            .collect();
        headers.sort();
        record.lock().unwrap().extend(headers);
        // The first protocol offered is chosen.
        if let Some(offered) = asked.headers().get("sec-websocket-protocol") {
            let first = offered.to_str().unwrap().split(',').next().unwrap().trim();
            answered
                .headers_mut()
                .insert("sec-websocket-protocol", first.parse().unwrap());
        }
        Ok(answered)
    };
    let Ok(mut socket) = tungstenite::accept_hdr(stream, answer) else {
        return;
    };
    match behaviour {
        Behaviour::Closes => {
            let _ = socket.close(Some(CloseFrame {
                code: CloseCode::Library(4001),
                reason: "the server says bye".into(),
            }));
        }
        Behaviour::Drops => return,
        Behaviour::Counts => {
            for n in 0..200 {
                socket.send(Message::text(n.to_string())).unwrap();
            }
        }
        Behaviour::Echo => {}
    }
    loop {
        match socket.read() {
            Ok(Message::Text(text)) => {
                seen.lock().unwrap().push(format!("text {text}"));
                let _ = socket.send(Message::Text(text));
            }
            Ok(Message::Binary(bytes)) => {
                seen.lock()
                    .unwrap()
                    .push(format!("binary {:?}", &bytes[..]));
                let _ = socket.send(Message::Binary(bytes));
            }
            Ok(Message::Ping(_)) => seen.lock().unwrap().push("ping".into()),
            Ok(Message::Close(frame)) => {
                let frame = frame.map(|frame| (u16::from(frame.code), frame.reason.to_string()));
                seen.lock().unwrap().push(format!("close {frame:?}"));
            }
            Ok(_) => {}
            Err(_) => return,
        }
    }
}

fn network() -> Arc<Network> {
    Arc::new(Network::new().unwrap())
}

fn connect(id: i64, url: String) -> Connect {
    Connect {
        id,
        url,
        protocols: Vec::new(),
        headers: Vec::new(),
    }
}

/// Events until one ends socket `id`, or `timeout` passes, waiting on the sockets' descriptor.
fn events(sockets: &mut WebSockets, id: i64, timeout: Duration) -> Vec<Event> {
    events_until(sockets, timeout, |event| event.id() == id && event.ends())
}

fn events_until(
    sockets: &mut WebSockets,
    timeout: Duration,
    done: impl Fn(&Event) -> bool,
) -> Vec<Event> {
    let deadline = Instant::now() + timeout;
    let mut got = Vec::new();
    loop {
        while let Some(event) = sockets.event() {
            let finished = done(&event);
            got.push(event);
            if finished {
                return got;
            }
        }
        let Some(wait) = deadline.checked_duration_since(Instant::now()) else {
            return got;
        };
        let mut fds = [PollFd::new(sockets, PollFlags::IN)];
        let _ = poll(&mut fds, Timespec::try_from(wait).ok().as_ref());
    }
}

fn opened(sockets: &mut WebSockets, id: i64) -> Event {
    let got = events_until(sockets, Duration::from_secs(5), |event| event.id() == id);
    got.into_iter().last().expect("an event")
}

#[test]
fn text_binary_ping_and_a_close_with_its_code_round_trip() {
    let (port, seen) = server(Behaviour::Echo, None);
    let mut sockets = WebSockets::new(network(), options(None, true)).unwrap();
    sockets
        .connect(connect(1, format!("ws://127.0.0.1:{port}/echo")))
        .unwrap();
    assert_eq!(
        opened(&mut sockets, 1),
        Event::Open {
            id: 1,
            protocol: String::new()
        }
    );
    sockets.send_text(1, "hola ñandú".into());
    sockets.send_binary(1, vec![0, 1, 254, 255]);
    sockets.ping(1);
    let got = events_until(&mut sockets, Duration::from_secs(5), |event| {
        matches!(event, Event::Binary { .. })
    });
    assert_eq!(
        got,
        [
            Event::Text {
                id: 1,
                text: "hola ñandú".into()
            },
            Event::Binary {
                id: 1,
                bytes: vec![0, 1, 254, 255]
            },
        ]
    );
    sockets.close(1, 4000, "done here").unwrap();
    // Closing: nothing more is sent, as Android's module refuses it.
    sockets.send_text(1, "late".into());
    let got = events_until(&mut sockets, Duration::from_secs(5), |event| {
        matches!(event, Event::Closed { .. })
    });
    assert_eq!(
        got,
        [
            Event::Failed {
                id: 1,
                message: "the WebSocket is not open".into()
            },
            Event::Closed {
                id: 1,
                code: 0,
                reason: "the WebSocket is not open".into()
            },
        ]
    );
    let deadline = Instant::now() + Duration::from_secs(2);
    while !seen
        .lock()
        .unwrap()
        .iter()
        .any(|line| line.starts_with("close"))
        && Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(10));
    }
    let seen = seen.lock().unwrap();
    let messages: Vec<&str> = seen
        .iter()
        .filter(|line| !line.contains(": "))
        .map(String::as_str)
        .collect();
    assert_eq!(
        messages,
        [
            "text hola ñandú",
            "binary [0, 1, 254, 255]",
            "ping",
            "close Some((4000, \"done here\"))"
        ]
    );
}

#[test]
fn the_close_handshake_ends_with_the_servers_answer() {
    let (port, _) = server(Behaviour::Echo, None);
    let mut sockets = WebSockets::new(network(), options(None, true)).unwrap();
    sockets
        .connect(connect(2, format!("ws://127.0.0.1:{port}/")))
        .unwrap();
    assert!(matches!(opened(&mut sockets, 2), Event::Open { .. }));
    sockets.close(2, 1000, "").unwrap();
    assert_eq!(
        events(&mut sockets, 2, Duration::from_secs(5)),
        [Event::Closed {
            id: 2,
            code: 1000,
            reason: String::new()
        }]
    );
}

#[test]
fn a_close_the_server_starts_is_answered_and_told_with_its_code() {
    let (port, _) = server(Behaviour::Closes, None);
    let mut sockets = WebSockets::new(network(), options(None, true)).unwrap();
    sockets
        .connect(connect(3, format!("ws://127.0.0.1:{port}/")))
        .unwrap();
    let got = events(&mut sockets, 3, Duration::from_secs(5));
    assert_eq!(
        got,
        [
            Event::Open {
                id: 3,
                protocol: String::new()
            },
            Event::Closed {
                id: 3,
                code: 4001,
                reason: "the server says bye".into()
            }
        ]
    );
}

#[test]
fn a_connection_lost_without_a_close_fails() {
    let (port, _) = server(Behaviour::Drops, None);
    let mut sockets = WebSockets::new(network(), options(None, true)).unwrap();
    sockets
        .connect(connect(4, format!("ws://127.0.0.1:{port}/")))
        .unwrap();
    let got = events(&mut sockets, 4, Duration::from_secs(5));
    assert!(matches!(got[0], Event::Open { .. }), "{got:?}");
    assert!(
        matches!(&got[1], Event::Failed { id: 4, message } if message.contains("without a close frame")),
        "{got:?}"
    );
}

#[test]
fn messages_arrive_in_order() {
    let (port, _) = server(Behaviour::Counts, None);
    let mut sockets = WebSockets::new(network(), options(None, true)).unwrap();
    sockets
        .connect(connect(5, format!("ws://127.0.0.1:{port}/")))
        .unwrap();
    let got = events_until(&mut sockets, Duration::from_secs(5), |event| {
        *event
            == Event::Text {
                id: 5,
                text: "199".into(),
            }
    });
    let texts: Vec<String> = got
        .into_iter()
        .filter_map(|event| match event {
            Event::Text { text, .. } => Some(text),
            _ => None,
        })
        .collect();
    assert_eq!(texts, (0..200).map(|n| n.to_string()).collect::<Vec<_>>());
}

#[test]
fn wss_round_trips_over_a_verified_tls() {
    let authority = authority();
    let (port, _) = server(Behaviour::Echo, Some(Arc::new(authority.server)));
    let mut sockets = WebSockets::new(network(), options(Some(authority.root), true)).unwrap();
    sockets
        .connect(connect(6, format!("wss://localhost:{port}/secure")))
        .unwrap();
    assert!(matches!(opened(&mut sockets, 6), Event::Open { .. }));
    sockets.send_text(6, "over TLS".into());
    sockets.send_binary(6, vec![7; 70_000]);
    let got = events_until(&mut sockets, Duration::from_secs(5), |event| {
        matches!(event, Event::Binary { .. })
    });
    assert_eq!(
        got,
        [
            Event::Text {
                id: 6,
                text: "over TLS".into()
            },
            Event::Binary {
                id: 6,
                bytes: vec![7; 70_000]
            }
        ]
    );
    sockets.close(6, 1000, "bye").unwrap();
    assert_eq!(
        events(&mut sockets, 6, Duration::from_secs(5)),
        [Event::Closed {
            id: 6,
            code: 1000,
            reason: "bye".into()
        }]
    );
}

#[test]
fn a_certificate_the_system_does_not_trust_fails() {
    let authority = authority();
    let (port, _) = server(Behaviour::Echo, Some(Arc::new(authority.server)));
    // The system's certificates only: the tests' authority is not among them.
    let mut sockets = WebSockets::new(network(), options(None, true)).unwrap();
    sockets
        .connect(connect(7, format!("wss://localhost:{port}/")))
        .unwrap();
    let got = events(&mut sockets, 7, Duration::from_secs(5));
    assert!(
        matches!(&got[..], [Event::Failed { id: 7, message }] if message.contains("certificate")),
        "{got:?}"
    );
}

#[test]
fn offline_a_socket_fails_at_once_but_one_to_this_machine_opens() {
    let (port, _) = server(Behaviour::Echo, None);
    let mut sockets = WebSockets::new(network(), options(None, false)).unwrap();
    let started = Instant::now();
    sockets
        .connect(connect(8, "wss://realtime.oxy.so/socket".into()))
        .unwrap();
    assert_eq!(
        events(&mut sockets, 8, Duration::from_secs(5)),
        [Event::Failed {
            id: 8,
            message: "the network is offline".into()
        }]
    );
    assert!(started.elapsed() < Duration::from_secs(1));
    sockets
        .connect(connect(9, format!("ws://localhost:{port}/")))
        .unwrap();
    assert!(matches!(opened(&mut sockets, 9), Event::Open { .. }));
}

#[test]
fn the_handshake_sends_protocols_headers_origin_and_the_networks_cookies() {
    // A server that sets a cookie over HTTP, so the network's jar holds it.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let http = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut buffer = [0u8; 4096];
            let _ = stream.read(&mut buffer);
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\nSet-Cookie: session=abc; Path=/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
        }
    });
    let network = network();
    network
        .fetch(
            crate::Request {
                method: "GET".into(),
                url: format!("http://{http}/"),
                with_credentials: true,
                ..crate::Request::default()
            },
            &|_| Ok(()),
        )
        .unwrap();
    let (port, seen) = server(Behaviour::Echo, None);
    let mut sockets = WebSockets::new(network, options(None, true)).unwrap();
    let mut asked = connect(10, format!("http://127.0.0.1:{port}/chat?room=1#ignored"));
    asked.protocols = vec![" chat ".into(), "".into(), "a,b".into(), "superchat".into()];
    asked.headers = vec![
        ("X-Oxy".into(), "yes".into()),
        (
            "Sec-WebSocket-Extensions".into(),
            "permessage-deflate".into(),
        ),
    ];
    sockets.connect(asked).unwrap();
    assert_eq!(
        opened(&mut sockets, 10),
        Event::Open {
            id: 10,
            protocol: "chat".into()
        }
    );
    let seen = seen.lock().unwrap().clone();
    for wanted in [
        "cookie: session=abc".to_string(),
        format!("origin: http://127.0.0.1:{port}"),
        "sec-websocket-protocol: chat,superchat".into(),
        "x-oxy: yes".into(),
    ] {
        assert!(seen.contains(&wanted), "{wanted} not in {seen:?}");
    }
    assert!(!seen
        .iter()
        .any(|line| line.starts_with("sec-websocket-extensions")));
    // The caller's origin wins.
    let (port, seen) = server(Behaviour::Echo, None);
    let mut asked = connect(11, format!("ws://127.0.0.1:{port}/"));
    asked.headers = vec![("Origin".into(), "https://oxy.so".into())];
    sockets.connect(asked).unwrap();
    assert!(matches!(opened(&mut sockets, 11), Event::Open { .. }));
    let seen = seen.lock().unwrap().clone();
    assert!(
        seen.contains(&"origin: https://oxy.so".to_string()),
        "{seen:?}"
    );
}

#[test]
fn malformed_sockets_are_refused_at_once() {
    let mut sockets = WebSockets::new(network(), options(None, true)).unwrap();
    assert!(sockets.connect(connect(1, "not a url".into())).is_err());
    assert!(sockets.connect(connect(1, "ftp://oxy.so/".into())).is_err());
    let mut bad = connect(1, "ws://127.0.0.1:1/".into());
    bad.headers = vec![("bad header".into(), "x".into())];
    assert!(sockets.connect(bad).is_err());
    assert!(sockets.event().is_none());
    assert_eq!(close_code_problem(1000), None);
    assert_eq!(close_code_problem(1001), None);
    assert_eq!(close_code_problem(4999), None);
    for code in [0, 999, 1004, 1005, 1006, 1015, 2999, 5000] {
        assert!(close_code_problem(code).is_some(), "{code}");
    }
    assert!(sockets.close(1, 1006, "").is_err());
    assert!(sockets.close(1, 1000, &"x".repeat(124)).is_err());
}

#[test]
fn a_socket_not_open_ends_when_used() {
    let mut sockets = WebSockets::new(network(), options(None, true)).unwrap();
    sockets.ping(42);
    assert_eq!(
        events_until(&mut sockets, Duration::from_secs(1), |event| {
            matches!(event, Event::Closed { .. })
        }),
        [
            Event::Failed {
                id: 42,
                message: "the WebSocket is not open".into()
            },
            Event::Closed {
                id: 42,
                code: 0,
                reason: "the WebSocket is not open".into()
            }
        ]
    );
    // Closing one already gone does nothing, as on the web.
    sockets.close(42, 1000, "").unwrap();
    assert!(sockets.event().is_none());
}

#[test]
fn closed_while_opening_it_fails_at_once() {
    // A server that accepts the connection and never answers the handshake.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        let held: Vec<_> = listener.incoming().take(1).collect();
        std::thread::sleep(Duration::from_secs(30));
        drop(held);
    });
    let mut sockets = WebSockets::new(network(), options(None, true)).unwrap();
    sockets
        .connect(connect(12, format!("ws://127.0.0.1:{port}/")))
        .unwrap();
    std::thread::sleep(Duration::from_millis(200));
    sockets.close(12, 1000, "").unwrap();
    assert_eq!(
        events(&mut sockets, 12, Duration::from_secs(2)),
        [Event::Failed {
            id: 12,
            message: "closed before it opened".into()
        }]
    );
    // Its thread stopped: nothing more is heard.
    std::thread::sleep(Duration::from_millis(200));
    assert!(sockets.event().is_none());
    assert_eq!(*sockets.shared.running.lock().unwrap(), 0);
}

#[test]
fn closed_once_it_opened_but_before_its_opening_was_heard_it_fails_and_closes() {
    let (port, seen) = server(Behaviour::Echo, None);
    let mut sockets = WebSockets::new(network(), options(None, true)).unwrap();
    sockets
        .connect(connect(19, format!("ws://127.0.0.1:{port}/")))
        .unwrap();
    // Opened, its opening queued and not taken.
    let deadline = Instant::now() + Duration::from_secs(5);
    while sockets.shared.events.lock().unwrap().is_empty() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    sockets.close(19, 1000, "").unwrap();
    assert_eq!(
        events(&mut sockets, 19, Duration::from_secs(2)),
        [Event::Failed {
            id: 19,
            message: "closed before it opened".into()
        }]
    );
    // The connection it had is closed, going away, and its thread ends.
    let wanted = "close Some((1001, \"\"))".to_string();
    let deadline = Instant::now() + Duration::from_secs(2);
    while !seen.lock().unwrap().contains(&wanted) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(seen.lock().unwrap().contains(&wanted), "{seen:?}");
    while *sockets.shared.running.lock().unwrap() > 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(*sockets.shared.running.lock().unwrap(), 0);
}

#[test]
fn a_server_that_never_answers_times_out_opening() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        let held: Vec<_> = listener.incoming().take(1).collect();
        std::thread::sleep(Duration::from_secs(30));
        drop(held);
    });
    let mut sockets = WebSockets::new(network(), options(None, true)).unwrap();
    sockets
        .connect(connect(13, format!("ws://127.0.0.1:{port}/")))
        .unwrap();
    let got = events(&mut sockets, 13, OPEN_TIMEOUT + Duration::from_secs(3));
    assert!(
        matches!(&got[..], [Event::Failed { message, .. }] if message.contains("timed out")),
        "{got:?}"
    );
}

#[test]
fn a_refused_connection_fails() {
    // A port bound and never listened on, which no other test can take meanwhile: refused.
    use rustix::net::{AddressFamily, SocketType};
    let held = rustix::net::socket(AddressFamily::INET, SocketType::STREAM, None).unwrap();
    rustix::net::bind(&held, &"127.0.0.1:0".parse::<SocketAddr>().unwrap()).unwrap();
    let bound: SocketAddr = rustix::net::getsockname(&held).unwrap().try_into().unwrap();
    let port = bound.port();
    let mut sockets = WebSockets::new(network(), options(None, true)).unwrap();
    sockets
        .connect(connect(14, format!("ws://127.0.0.1:{port}/")))
        .unwrap();
    let got = events(&mut sockets, 14, Duration::from_secs(5));
    assert!(
        matches!(&got[..], [Event::Failed { message, .. }] if message.contains("refused")),
        "{got:?}"
    );
}

#[test]
fn a_server_that_refuses_the_upgrade_fails_with_its_status() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut buffer = [0u8; 4096];
            let _ = stream.read(&mut buffer);
            let _ = stream.write_all(
                b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
        }
    });
    let mut sockets = WebSockets::new(network(), options(None, true)).unwrap();
    sockets
        .connect(connect(15, format!("ws://127.0.0.1:{port}/")))
        .unwrap();
    assert_eq!(
        events(&mut sockets, 15, Duration::from_secs(5)),
        [Event::Failed {
            id: 15,
            message: "Expected HTTP 101 response but was '403 Forbidden'".into()
        }]
    );
}

#[test]
fn dropped_with_the_runtime_open_sockets_close_going_away() {
    let (port, seen) = server(Behaviour::Echo, None);
    let mut sockets = WebSockets::new(network(), options(None, true)).unwrap();
    sockets
        .connect(connect(16, format!("ws://127.0.0.1:{port}/")))
        .unwrap();
    assert!(matches!(opened(&mut sockets, 16), Event::Open { .. }));
    let shared = sockets.shared.clone();
    drop(sockets);
    // The close frame was sent before the drop returned, and the thread ended.
    assert_eq!(*shared.running.lock().unwrap(), 0);
    let deadline = Instant::now() + Duration::from_secs(2);
    let wanted = "close Some((1001, \"\"))".to_string();
    while !seen.lock().unwrap().contains(&wanted) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(seen.lock().unwrap().contains(&wanted), "{seen:?}");
}

#[test]
fn a_proxy_tunnels_the_socket() {
    let (port, _) = server(Behaviour::Echo, None);
    // An HTTP proxy that tunnels CONNECT to the server and says what it was asked.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let proxy = listener.local_addr().unwrap();
    let asked: Seen = Arc::default();
    let record = asked.clone();
    std::thread::spawn(move || {
        for mut client in listener.incoming().flatten() {
            let mut head = Vec::new();
            let mut byte = [0u8];
            while !head.ends_with(b"\r\n\r\n") && client.read(&mut byte).unwrap() == 1 {
                head.push(byte[0]);
            }
            let head = String::from_utf8(head).unwrap();
            record
                .lock()
                .unwrap()
                .push(head.lines().next().unwrap().to_string());
            let target = head.split_whitespace().nth(1).unwrap().to_string();
            let mut server = TcpStream::connect(target).unwrap();
            client
                .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
                .unwrap();
            let (mut up, mut down) = (client.try_clone().unwrap(), server.try_clone().unwrap());
            std::thread::spawn(move || io::copy(&mut up, &mut down));
            std::thread::spawn(move || io::copy(&mut server, &mut client));
        }
    });
    let mut through = options(None, true);
    through.proxy = Some(ureq::Proxy::new(&format!("http://{proxy}")).unwrap());
    let mut sockets = WebSockets::new(network(), through).unwrap();
    sockets
        .connect(connect(17, format!("ws://localhost:{port}/")))
        .unwrap();
    assert!(matches!(opened(&mut sockets, 17), Event::Open { .. }));
    sockets.send_text(17, "through the proxy".into());
    assert_eq!(
        events_until(&mut sockets, Duration::from_secs(5), |_| true),
        [Event::Text {
            id: 17,
            text: "through the proxy".into()
        }]
    );
    assert_eq!(
        *asked.lock().unwrap(),
        [format!("CONNECT localhost:{port} HTTP/1.1")]
    );
}

#[test]
fn an_address_off_the_machine_is_not_local() {
    assert!(still_local("127.0.0.1:443".parse().unwrap()));
    assert!(still_local("[::1]:443".parse().unwrap()) || !still_local("[::1]:0".parse().unwrap()));
    // TEST-NET-1 (RFC 5737) is never a machine's.
    assert!(!still_local("192.0.2.1:443".parse().unwrap()));
}

/// The context switches of this process's thread named `name`, if one runs.
fn switches(name: &str) -> Option<u64> {
    for task in std::fs::read_dir("/proc/self/task").ok()?.flatten() {
        let status = std::fs::read_to_string(task.path().join("status")).unwrap_or_default();
        if status.lines().any(|line| line == format!("Name:\t{name}")) {
            return Some(
                status
                    .lines()
                    .filter(|line| line.contains("ctxt_switches"))
                    .filter_map(|line| line.split_whitespace().nth(1)?.parse::<u64>().ok())
                    .sum(),
            );
        }
    }
    None
}

#[test]
fn an_open_idle_socket_wakes_nothing() {
    let (port, _) = server(Behaviour::Echo, None);
    let mut sockets = WebSockets::new(network(), options(None, true)).unwrap();
    sockets
        .connect(connect(918, format!("ws://127.0.0.1:{port}/")))
        .unwrap();
    assert!(matches!(opened(&mut sockets, 918), Event::Open { .. }));
    std::thread::sleep(Duration::from_millis(100));
    let before = switches("oxyos-ws-918").expect("the socket's thread");
    std::thread::sleep(Duration::from_secs(2));
    let after = switches("oxyos-ws-918").expect("the socket's thread");
    // Nothing arrived and nothing was sent: its thread slept in poll.
    assert!(after - before <= 1, "woke {} times", after - before);
    assert!(sockets.event().is_none());
    // And it is still open.
    sockets.send_text(918, "still here".into());
    assert_eq!(
        events_until(&mut sockets, Duration::from_secs(5), |_| true),
        [Event::Text {
            id: 918,
            text: "still here".into()
        }]
    );
}
