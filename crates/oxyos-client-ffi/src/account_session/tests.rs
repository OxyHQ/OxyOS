use super::*;
use oxyos_account::ipc::Listener;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};

/// A directory of the test's own for a socket (a socket's path holds at most 107 bytes).
fn socket_path(name: &str) -> (PathBuf, PathBuf) {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "oxyos-ffi-session-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    (dir.join(name), dir)
}

/// A service answering each connection's messages with `answer(message)`, in turn, until it
/// returns None (the connection then ends); every message received is kept.
fn service(
    socket: &Path,
    connections: usize,
    answer: impl Fn(&Value) -> Vec<Value> + Send + 'static,
) -> std::thread::JoinHandle<Vec<Value>> {
    let listener = Listener::bind(socket).unwrap();
    std::thread::spawn(move || {
        let mut received = Vec::new();
        for _ in 0..connections {
            let connection = listener.accept().unwrap();
            while let Ok(Some(Some(message))) = connection.receive::<Value>() {
                let answers = answer(&message);
                received.push(message);
                if answers.is_empty() {
                    break;
                }
                for answer in answers {
                    connection.send(&answer).unwrap();
                }
            }
        }
        received
    })
}

fn signed_in(revision: u64) -> Value {
    json!({"snapshot": {"status": "signed_in", "revision": revision, "online": true,
        "account": {"id": "0190a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b", "handle": "nate", "displayName": "Nate"}}})
}

#[test]
fn a_snapshot_is_the_services_rebuilt_and_strict() {
    let (socket, dir) = socket_path("session");
    let shapes = [
        signed_in(3),
        json!({"snapshot": {"status": "signed_in", "revision": 4, "online": false, "account": null}}),
        json!({"snapshot": {"status": "signed_out", "revision": 5, "online": true, "account": null, "secret": "x"}}),
        json!({"refused": {"reason": "notPermitted"}}),
    ];
    let answers = std::sync::Mutex::new(shapes.into_iter());
    let served = service(&socket, 4, move |message| {
        assert_eq!(message, &json!("snapshot"));
        vec![answers.lock().unwrap().next().unwrap()]
    });
    let snapshot = snapshot_at(&socket).unwrap();
    assert_eq!(snapshot.status, Status::SignedIn);
    assert_eq!(
        serde_json::to_value(&snapshot).unwrap(),
        json!({"status": "signed_in", "revision": 3, "online": true,
            "account": {"id": "0190a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b", "handle": "nate", "displayName": "Nate"}})
    );
    assert!(snapshot_at(&socket).is_err(), "signed in names an account");
    assert!(
        snapshot_at(&socket).is_err(),
        "nothing but a snapshot's keys"
    );
    assert_eq!(
        snapshot_at(&socket),
        Err(SessionError::Refused("notPermitted".into()))
    );
    served.join().unwrap();
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn no_service_no_session() {
    let (socket, dir) = socket_path("session");
    assert!(matches!(snapshot_at(&socket), Err(SessionError::Failed(_))));
    assert!(matches!(
        token_at(&socket, TokenReason::Boot),
        Err(SessionError::Failed(_))
    ));
    assert!(Subscription::connect(&socket).is_err());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_token_names_its_reason_and_is_never_printed() {
    let (socket, dir) = socket_path("session");
    let served = service(&socket, 3, |message| {
        let reason = message["token"]["reason"].as_str().unwrap().to_owned();
        vec![match reason.as_str() {
            "boot" => {
                json!({"token": {"accessToken": "eyJ.secret.token", "expiresAt": "2026-11-06T12:05:00.000Z"}})
            }
            "preflight" => json!({"token": null}),
            "response-401" => json!({"token": {"accessToken": "", "expiresAt": "x"}}),
            _ => json!({"refused": {"reason": "invalidRequest"}}),
        }]
    });
    let token = token_at(&socket, TokenReason::Boot).unwrap().unwrap();
    assert_eq!(token.access_token.expose(), "eyJ.secret.token");
    assert_eq!(token.expires_at, "2026-11-06T12:05:00.000Z");
    assert!(!format!("{token:?}").contains("secret"));
    assert!(token_at(&socket, TokenReason::Preflight).unwrap().is_none());
    let error = token_at(&socket, TokenReason::Response401).unwrap_err();
    assert!(
        matches!(error, SessionError::Failed(_)),
        "an empty token is none"
    );
    // Through the C ABI: the reason checked, the token written whole.
    let mut out = vec![0u8; 16 * 1024];
    #[allow(unsafe_code)]
    // SAFETY: a NUL-terminated reason and a buffer of its length.
    let refused = unsafe {
        oxyos_account_session_token(c"later".as_ptr(), out.as_mut_ptr().cast(), out.len())
    };
    assert!(refused < 0);
    served.join().unwrap();
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_subscription_hands_each_snapshot_once_and_says_when_it_ends() {
    let (socket, dir) = socket_path("session");
    let (go, wait) = std::sync::mpsc::channel::<()>();
    let wait = std::sync::Mutex::new(wait);
    let listener = Listener::bind(&socket).unwrap();
    let served = std::thread::spawn(move || {
        let connection = listener.accept().unwrap();
        assert_eq!(
            connection.receive::<Value>().unwrap().unwrap().unwrap(),
            json!("subscribe")
        );
        connection.send(&signed_in(1)).unwrap();
        wait.lock().unwrap().recv().unwrap();
        connection.send(&signed_in(2)).unwrap();
        connection
            .send(&json!({"snapshot": {"status": "signed_out", "revision": 3, "online": true, "account": null}}))
            .unwrap();
        wait.lock().unwrap().recv().unwrap();
    });
    let mut subscription = Subscription::connect(&socket).unwrap();
    let first = loop {
        if let Some(json) = subscription.arrived().unwrap() {
            break json;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    assert!(first.contains("\"revision\":1"));
    assert_eq!(subscription.arrived().unwrap(), None, "nothing new yet");
    go.send(()).unwrap();
    let mut seen = Vec::new();
    while seen.len() < 2 {
        if let Some(json) = subscription.arrived().unwrap() {
            seen.push(json);
        }
    }
    assert!(seen[0].contains("\"revision\":2"));
    assert!(seen[1].contains("signed_out"));
    go.send(()).unwrap();
    served.join().unwrap();
    let ended = loop {
        match subscription.arrived() {
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
            other => break other,
        }
    };
    assert!(
        ended.is_err(),
        "the service went away: the subscription ended"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn removing_is_answered_or_refused() {
    let (socket, dir) = socket_path("session");
    let answers = std::sync::Mutex::new(
        vec![
            json!("removed"),
            json!({"refused": {"reason": "notPermitted"}}),
        ]
        .into_iter(),
    );
    let served = service(&socket, 2, move |message| {
        assert_eq!(message, &json!({"remove": {}}));
        vec![answers.lock().unwrap().next().unwrap()]
    });
    assert_eq!(remove_at(&socket), Ok(()));
    assert_eq!(
        remove_at(&socket),
        Err(SessionError::Refused("notPermitted".into()))
    );
    served.join().unwrap();
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn the_platform_is_the_services() {
    let (socket, dir) = socket_path("accountd");
    let answers = std::sync::Mutex::new(
        vec![
            json!({"platform": {"api": "https://api.oxy.so", "clientId": "oxy_pk_public"}}),
            json!({"refused": {"reason": "notConfigured"}}),
            json!({"platform": {"api": "https://api.oxy.so", "clientId": ""}}),
            json!({"platform": {"api": "https://api.oxy.so", "clientId": null}}),
        ]
        .into_iter(),
    );
    let served = service(&socket, 4, move |message| {
        assert_eq!(message, &json!("platform"));
        vec![answers.lock().unwrap().next().unwrap()]
    });
    assert_eq!(
        platform_at(&socket).unwrap(),
        Platform {
            api: "https://api.oxy.so".into(),
            client_id: "oxy_pk_public".into()
        }
    );
    assert_eq!(
        platform_at(&socket),
        Err(SessionError::Refused("notConfigured".into()))
    );
    assert_eq!(
        platform_at(&socket),
        Err(SessionError::Refused("notConfigured".into())),
        "no client id, no platform"
    );
    assert_eq!(
        platform_at(&socket),
        Err(SessionError::Refused("notConfigured".into()))
    );
    served.join().unwrap();
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_setup_answers_a_second_factor_on_its_connection_and_keeps_no_secret() {
    let (socket, dir) = socket_path("accountd");
    let served = service(&socket, 1, |message| {
        if message.get("enroll").is_some() {
            vec![json!("secondFactorRequired")]
        } else if message["secondFactor"]["code"] == "000000" {
            vec![json!({"refused": {"reason": "secondFactorInvalid"}})]
        } else {
            vec![
                json!({"enrolled": {"accountId": "a1", "user": "oxy-abc", "uid": 1001,
                "profile": {"id": "a1", "username": "nate", "displayName": null, "avatar": null}}}),
            ]
        }
    });
    let mut enrollment = Enrollment::connect(&socket).unwrap();
    assert!(
        enrollment.second_factor(&Secret::new("1")).is_err(),
        "nothing sent yet"
    );
    assert_eq!(
        enrollment
            .enroll(
                "device-1",
                &Secret::new("device-secret"),
                "a1",
                &Secret::new("hunter22")
            )
            .unwrap(),
        "\"secondFactorRequired\""
    );
    let wrong = enrollment.second_factor(&Secret::new("000000")).unwrap();
    assert!(wrong.contains("secondFactorInvalid"));
    let enrolled = enrollment
        .second_factor(&Secret::new("123456"))
        .unwrap()
        .to_owned();
    assert_eq!(
        serde_json::from_str::<Value>(&enrolled).unwrap(),
        json!({"enrolled": {"accountId": "a1", "user": "oxy-abc"}})
    );
    assert!(
        enrollment.second_factor(&Secret::new("123456")).is_err(),
        "an ended setup sends nothing more"
    );
    let received = served.join().unwrap();
    assert_eq!(
        received[0],
        json!({"enroll": {"deviceId": "device-1", "deviceSecret": "device-secret", "accountId": "a1", "password": "hunter22"}})
    );
    assert_eq!(received[1], json!({"secondFactor": {"code": "000000"}}));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_refused_setup_ends_it() {
    let (socket, dir) = socket_path("accountd");
    let served = service(&socket, 1, |_| {
        vec![json!({"refused": {"reason": "locked", "retryAfterSeconds": 60}})]
    });
    let mut enrollment = Enrollment::connect(&socket).unwrap();
    let answer = enrollment
        .enroll("d", &Secret::new("s"), "a", &Secret::new("p"))
        .unwrap()
        .to_owned();
    assert_eq!(
        serde_json::from_str::<Value>(&answer).unwrap(),
        json!({"refused": {"reason": "locked", "retryAfterSeconds": 60}})
    );
    assert!(enrollment.second_factor(&Secret::new("1")).is_err());
    served.join().unwrap();
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn the_header_names_a_message_the_services_size() {
    let header = include_str!("../../include/oxyos_client.h");
    let line = header
        .lines()
        .find(|line| line.starts_with("#define OXYOS_ACCOUNT_MESSAGE_MAX "))
        .expect("OXYOS_ACCOUNT_MESSAGE_MAX is defined");
    let value: usize = line.rsplit(' ').next().unwrap().parse().unwrap();
    assert_eq!(value, oxyos_account::ipc::MAX_MESSAGE);
}
