use super::*;
use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use std::time::Instant;

/// A server answering the tests' routes, on a port of its own; returns its base URL.
fn server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            std::thread::spawn(move || {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let mut parts = line.split_whitespace();
                let method = parts.next().unwrap_or("").to_string();
                let path = parts.next().unwrap_or("").to_string();
                let mut length = 0;
                let mut cookie = String::new();
                let mut authorization = String::new();
                loop {
                    let mut header = String::new();
                    reader.read_line(&mut header).unwrap();
                    let header = header.trim_end();
                    if header.is_empty() {
                        break;
                    }
                    let (name, value) = header.split_once(':').unwrap();
                    match name.to_ascii_lowercase().as_str() {
                        "content-length" => length = value.trim().parse().unwrap(),
                        "cookie" => cookie = value.trim().to_string(),
                        "authorization" => authorization = value.trim().to_string(),
                        _ => {}
                    }
                }
                let mut body = vec![0u8; length];
                reader.read_exact(&mut body).unwrap();
                let mut stream = stream;
                let reply = |stream: &mut std::net::TcpStream,
                             status: &str,
                             extra: &str,
                             body: &str| {
                    let _ = write!(
                        stream,
                        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: text/plain\r\n{extra}Connection: close\r\n\r\n{body}",
                        body.len()
                    );
                };
                if let Some(target) = path.strip_prefix("/goto?") {
                    let location = format!("Location: {target}\r\n");
                    return reply(&mut stream, "302 Found", &location, "");
                }
                match path.as_str() {
                    "/json" => reply(
                        &mut stream,
                        "200 OK",
                        "X-Oxy: one\r\nX-Oxy: two\r\n",
                        "{\"a\":1}",
                    ),
                    "/echo" => {
                        let text = format!("{method} {}", String::from_utf8_lossy(&body));
                        reply(&mut stream, "200 OK", "", &text)
                    }
                    "/redirect" => reply(&mut stream, "302 Found", "Location: /json\r\n", ""),
                    "/slow" => {
                        std::thread::sleep(Duration::from_secs(2));
                        reply(&mut stream, "200 OK", "", "late")
                    }
                    "/set" => reply(
                        &mut stream,
                        "200 OK",
                        "Set-Cookie: oxy=yes; Path=/\r\n",
                        "set",
                    ),
                    "/cookie" => reply(&mut stream, "200 OK", "", &cookie),
                    "/set-and-go" => reply(
                        &mut stream,
                        "302 Found",
                        "Set-Cookie: hop=1; Path=/\r\nLocation: /cookie\r\n",
                        "",
                    ),
                    "/authorization" => reply(&mut stream, "200 OK", "", &authorization),
                    _ => reply(&mut stream, "404 Not Found", "", "missing"),
                }
            });
        }
    });
    format!("http://{address}")
}

fn request(id: i64, method: &str, url: String) -> Request {
    Request {
        id,
        method: method.to_string(),
        url,
        with_credentials: true,
        ..Request::default()
    }
}

/// Every answer of request `id` until its end, waiting at most `limit`.
fn answers(network: &Network, id: i64, limit: Duration) -> Vec<Answer> {
    let deadline = Instant::now() + limit;
    let mut got = Vec::new();
    while Instant::now() < deadline {
        match network.answer() {
            Some(answer) if answer.id() == id => {
                let end = matches!(answer, Answer::Complete { .. });
                got.push(answer);
                if end {
                    return got;
                }
            }
            Some(_) => {}
            None => std::thread::sleep(Duration::from_millis(5)),
        }
    }
    got
}

fn body(answers: &[Answer]) -> String {
    answers
        .iter()
        .filter_map(|answer| match answer {
            Answer::Data { bytes, .. } | Answer::Chunk { bytes, .. } => {
                Some(String::from_utf8_lossy(bytes).into_owned())
            }
            _ => None,
        })
        .collect()
}

fn completed(answers: &[Answer]) -> (Option<String>, bool) {
    match answers.last() {
        Some(Answer::Complete {
            error, timed_out, ..
        }) => (error.clone(), *timed_out),
        other => panic!("no end: {other:?}"),
    }
}

#[test]
fn a_response_comes_with_its_status_headers_body_and_end() {
    let base = server();
    let network = Network::new().unwrap();
    network.send(request(1, "GET", format!("{base}/json")));
    let got = answers(&network, 1, Duration::from_secs(5));
    let Answer::Response {
        status, headers, ..
    } = &got[0]
    else {
        panic!("{got:?}")
    };
    assert_eq!(*status, 200);
    assert!(
        headers.contains(&("x-oxy".to_string(), "one, two".to_string())),
        "{headers:?}"
    );
    assert!(matches!(&got[1], Answer::Data { content_type: Some(t), .. } if t == "text/plain"));
    assert_eq!(body(&got), "{\"a\":1}");
    assert_eq!(completed(&got), (None, false));
    // A status is an answer, not a failure.
    network.send(request(2, "GET", format!("{base}/nowhere")));
    let got = answers(&network, 2, Duration::from_secs(5));
    assert!(matches!(&got[0], Answer::Response { status: 404, .. }));
    assert_eq!(completed(&got), (None, false));
}

#[test]
fn a_body_is_sent_and_redirects_end_where_they_lead() {
    let base = server();
    let network = Network::new().unwrap();
    let mut post = request(1, "POST", format!("{base}/echo"));
    post.body = Some(b"hola".to_vec());
    network.send(post);
    assert_eq!(
        body(&answers(&network, 1, Duration::from_secs(5))),
        "POST hola"
    );
    network.send(request(2, "GET", format!("{base}/redirect")));
    let got = answers(&network, 2, Duration::from_secs(5));
    assert!(matches!(&got[0], Answer::Response { status: 200, url, .. } if url.ends_with("/json")));
    assert_eq!(body(&got), "{\"a\":1}");
}

#[test]
fn an_incremental_body_comes_in_chunks_with_its_progress() {
    let base = server();
    let network = Network::new().unwrap();
    let mut incremental = request(1, "GET", format!("{base}/json"));
    incremental.incremental = true;
    network.send(incremental);
    let got = answers(&network, 1, Duration::from_secs(5));
    assert!(got
        .iter()
        .all(|answer| !matches!(answer, Answer::Data { .. })));
    assert!(matches!(
        got.iter()
            .find(|answer| matches!(answer, Answer::Chunk { .. })),
        Some(Answer::Chunk {
            progress: 7,
            total: Some(7),
            ..
        })
    ));
    assert_eq!(body(&got), "{\"a\":1}");
}

#[test]
fn a_timeout_a_refusal_and_an_abort_are_ends() {
    let base = server();
    let network = Network::new().unwrap();
    let mut slow = request(1, "GET", format!("{base}/slow"));
    slow.timeout = Some(Duration::from_millis(200));
    network.send(slow);
    let (error, timed_out) = completed(&answers(&network, 1, Duration::from_secs(5)));
    assert!(error.is_some() && timed_out);
    // A port that was just closed refuses at once.
    let closed = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    network.send(request(2, "GET", format!("http://{closed}/")));
    let (error, timed_out) = completed(&answers(&network, 2, Duration::from_secs(5)));
    assert!(error.is_some() && !timed_out);
    // An aborted request delivers nothing, not even its end.
    network.send(request(3, "GET", format!("{base}/slow")));
    network.abort(3);
    assert!(answers(&network, 3, Duration::from_secs(3)).is_empty());
    // Nothing is kept of it, nor of an abort after the end.
    network.abort(1);
    assert!(network.shared.heard.lock().unwrap().is_empty());
}

#[test]
fn cookies_are_kept_sent_back_and_cleared() {
    let base = server();
    let network = Network::new().unwrap();
    network.send(request(1, "GET", format!("{base}/set")));
    answers(&network, 1, Duration::from_secs(5));
    network.send(request(2, "GET", format!("{base}/cookie")));
    assert_eq!(
        body(&answers(&network, 2, Duration::from_secs(5))),
        "oxy=yes"
    );
    // Without credentials, the jar stays out of it.
    let mut anonymous = request(3, "GET", format!("{base}/cookie"));
    anonymous.with_credentials = false;
    network.send(anonymous);
    assert_eq!(body(&answers(&network, 3, Duration::from_secs(5))), "");
    network.clear_cookies();
    network.send(request(4, "GET", format!("{base}/cookie")));
    assert_eq!(body(&answers(&network, 4, Duration::from_secs(5))), "");
}

#[test]
fn each_hop_of_a_redirect_keeps_its_cookies_and_the_method_changes_as_browsers_do() {
    let base = server();
    let network = Network::new().unwrap();
    // A cookie set on the way is sent where the redirect leads.
    network.send(request(1, "GET", format!("{base}/set-and-go")));
    assert_eq!(body(&answers(&network, 1, Duration::from_secs(5))), "hop=1");
    // A POST redirected with 302 goes on as a GET without its body.
    let mut post = request(2, "POST", format!("{base}/goto?/echo"));
    post.body = Some(b"hola".to_vec());
    network.send(post);
    assert_eq!(body(&answers(&network, 2, Duration::from_secs(5))), "GET ");
}

#[test]
fn credentials_are_not_handed_to_another_origin() {
    let base = server();
    let other = server();
    let network = Network::new().unwrap();
    let mut same = request(1, "GET", format!("{base}/goto?/authorization"));
    same.headers = vec![("Authorization".to_string(), "Bearer oxy".to_string())];
    network.send(same);
    assert_eq!(
        body(&answers(&network, 1, Duration::from_secs(5))),
        "Bearer oxy"
    );
    let mut away = request(2, "GET", format!("{base}/goto?{other}/authorization"));
    away.headers = vec![("Authorization".to_string(), "Bearer oxy".to_string())];
    network.send(away);
    assert_eq!(body(&answers(&network, 2, Duration::from_secs(5))), "");
}

/// A server that drops the first `dropped` connections once it has read their request, then
/// answers with the method and body of the request it got; returns its base URL.
fn dropping(dropped: usize) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for (index, stream) in listener.incoming().flatten().enumerate() {
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let mut length = 0;
            loop {
                let mut header = String::new();
                reader.read_line(&mut header).unwrap();
                if header.trim_end().is_empty() {
                    break;
                }
                if let Some((name, value)) = header.split_once(':') {
                    if name.eq_ignore_ascii_case("content-length") {
                        length = value.trim().parse().unwrap();
                    }
                }
            }
            let mut body = vec![0u8; length];
            reader.read_exact(&mut body).unwrap();
            if index < dropped {
                continue;
            }
            let text = format!(
                "{} {}",
                line.split_whitespace().next().unwrap(),
                String::from_utf8_lossy(&body)
            );
            let mut stream = stream;
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                text.len()
            );
        }
    });
    format!("http://{address}")
}

#[test]
fn a_connection_lost_before_the_response_is_tried_again_and_then_fails() {
    let network = Network::new().unwrap();
    // As on Android, even a POST: nothing was answered.
    let mut post = request(1, "POST", format!("{}/", dropping(2)));
    post.body = Some(b"hola".to_vec());
    network.send(post);
    let got = answers(&network, 1, Duration::from_secs(5));
    assert_eq!(body(&got), "POST hola");
    assert_eq!(completed(&got), (None, false));
    // A server that never answers fails the request once the retries are spent.
    network.send(request(2, "GET", format!("{}/", dropping(usize::MAX))));
    let (error, timed_out) = completed(&answers(&network, 2, Duration::from_secs(5)));
    assert!(error.is_some() && !timed_out);
}
