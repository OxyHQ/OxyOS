use super::*;
use oxyos_account_proto::ipc::Listener;
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
    let listener = Listener::bind(socket, oxyos_account_proto::ipc::MODE).unwrap();
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
        "account": {"id": "0190a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b", "handle": "nate", "displayName": "Nate",
            "languages": ["es-ES", "en-GB"]}}})
}

#[test]
fn a_snapshot_is_the_services_rebuilt_and_strict() {
    let (socket, dir) = socket_path("session");
    let shapes = [
        signed_in(3),
        json!({"snapshot": {"status": "signed_in", "revision": 4, "online": false, "account": null}}),
        json!({"snapshot": {"status": "signed_out", "revision": 5, "online": true, "account": {"id": "a", "handle": "b"}}}),
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
            "account": {"id": "0190a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b", "handle": "nate", "displayName": "Nate",
                "languages": ["es-ES", "en-GB"]}})
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
    let listener = Listener::bind(&socket, oxyos_account_proto::ipc::MODE).unwrap();
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
fn what_erasing_would_do_is_the_services_rebuilt_and_strict() {
    let (socket, dir) = socket_path("session");
    let preview = json!({"erasePreview": {"available": true, "administrator": true, "online": true,
        "accounts": [{"name": "Ada Lovelace", "handle": "ada", "signsOutOnline": true},
                     {"name": "oxy-x", "handle": null, "signsOutOnline": false}]}});
    let shapes = [
        preview.clone(),
        // Not one: no account, an unnamed one, one signed out online while offline, a field more.
        json!({"erasePreview": {"available": true, "administrator": true, "online": true, "accounts": []}}),
        json!({"erasePreview": {"available": true, "administrator": true, "online": true,
            "accounts": [{"name": " ", "handle": null, "signsOutOnline": false}]}}),
        json!({"erasePreview": {"available": true, "administrator": true, "online": false,
            "accounts": [{"name": "Ada", "handle": null, "signsOutOnline": true}]}}),
        json!({"erasePreview": {"available": true, "administrator": true, "online": true, "extra": 1,
            "accounts": [{"name": "Ada", "handle": null, "signsOutOnline": true}]}}),
        json!({"refused": {"reason": "notPermitted"}}),
    ];
    let count = shapes.len();
    let answers = std::sync::Mutex::new(shapes.into_iter());
    let served = service(&socket, count, move |message| {
        assert_eq!(message, &json!({"erasePreview": {}}));
        vec![answers.lock().unwrap().next().unwrap()]
    });
    let rebuilt = serde_json::to_value(erase_preview_at(&socket).unwrap()).unwrap();
    assert_eq!(rebuilt, preview["erasePreview"]);
    for _ in 0..4 {
        assert!(matches!(
            erase_preview_at(&socket),
            Err(SessionError::Failed(_))
        ));
    }
    assert_eq!(
        erase_preview_at(&socket),
        Err(SessionError::Refused("notPermitted".into()))
    );
    served.join().unwrap();
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn an_erase_is_answered_or_refused_with_its_reason_and_wait() {
    let (socket, dir) = socket_path("session");
    let shapes = [
        json!({"erasing": {"signedOut": ["Ada Lovelace"], "notSignedOut": ["Grace Hopper"]}}),
        json!({"refused": {"reason": "wrongPassword"}}),
        json!({"refused": {"reason": "throttled", "retryAfter": 60}}),
        json!({"refused": {"reason": "notPermitted"}}),
        json!("removed"),
    ];
    let count = shapes.len();
    let answers = std::sync::Mutex::new(shapes.into_iter());
    let served = service(&socket, count, move |message| {
        assert_eq!(message, &json!({"eraseAll": {"password": "correct horse"}}));
        vec![answers.lock().unwrap().next().unwrap()]
    });
    let erase = || {
        erase_all_at(&socket, Secret::new("correct horse"))
            .map(|answer| serde_json::to_value(answer).unwrap())
    };
    assert_eq!(
        erase(),
        Ok(json!({"erasing": {"signedOut": ["Ada Lovelace"], "notSignedOut": ["Grace Hopper"]}}))
    );
    assert_eq!(erase(), Ok(json!({"refused": {"reason": "wrongPassword"}})));
    assert_eq!(
        erase(),
        Ok(json!({"refused": {"reason": "throttled", "retryAfter": 60}}))
    );
    assert_eq!(erase(), Ok(json!({"refused": {"reason": "notPermitted"}})));
    let failed = erase().unwrap_err();
    assert!(!failed.to_string().contains("correct horse"));
    served.join().unwrap();
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_save_is_answered_saved_not_saved_or_refused() {
    let (socket, dir) = socket_path("session");
    let shapes = [
        json!({"saved": {}}),
        json!({"notSaved": {}}),
        json!({"refused": {"reason": "locked"}}),
        json!("removed"),
    ];
    let count = shapes.len();
    let answers = std::sync::Mutex::new(shapes.into_iter());
    let served = service(&socket, count, move |_| {
        vec![answers.lock().unwrap().next().unwrap()]
    });
    let save = |domain: Option<&str>| {
        save_password_at(
            &socket,
            Secret::new("ada"),
            Secret::new("made-up-pass"),
            domain.map(str::to_owned),
        )
        .map(|answer| serde_json::to_value(answer).unwrap())
    };
    assert_eq!(save(Some("dropsea.net")), Ok(json!({"saved": {}})));
    assert_eq!(save(None), Ok(json!({"notSaved": {}})));
    assert_eq!(save(None), Ok(json!({"refused": {"reason": "locked"}})));
    let failed = save(None).unwrap_err();
    assert!(!failed.to_string().contains("made-up-pass"));
    assert_eq!(
        served.join().unwrap(),
        [
            json!({"savePassword": {"username": "ada", "password": "made-up-pass", "domain": "dropsea.net"}}),
            json!({"savePassword": {"username": "ada", "password": "made-up-pass"}}),
            json!({"savePassword": {"username": "ada", "password": "made-up-pass"}}),
            json!({"savePassword": {"username": "ada", "password": "made-up-pass"}}),
        ]
    );
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
                Some(&Secret::new("hunter22")),
                Some(&Secret::new("482916"))
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
        json!({"enrolled": {"accountId": "a1", "user": "oxy-abc", "name": "nate"}})
    );
    assert!(
        enrollment.second_factor(&Secret::new("123456")).is_err(),
        "an ended setup sends nothing more"
    );
    let received = served.join().unwrap();
    assert_eq!(
        received[0],
        json!({"enroll": {"deviceId": "device-1", "deviceSecret": "device-secret", "accountId": "a1", "password": "hunter22", "pin": "482916"}})
    );
    assert_eq!(received[1], json!({"secondFactor": {"code": "000000"}}));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_refused_setup_ends_it() {
    let (socket, dir) = socket_path("accountd");
    let served = service(&socket, 1, |message| {
        // A sign-in by the QR: only its PIN is sent (refused here as the service refuses one it lacks).
        assert_eq!(
            message.clone(),
            json!({"enroll": {"deviceId": "d", "deviceSecret": "s", "accountId": "a", "pin": "4829"}})
        );
        vec![json!({"refused": {"reason": "pinRequired"}})]
    });
    let mut enrollment = Enrollment::connect(&socket).unwrap();
    let answer = enrollment
        .enroll(
            "d",
            &Secret::new("s"),
            "a",
            None,
            Some(&Secret::new("4829")),
        )
        .unwrap()
        .to_owned();
    assert_eq!(
        serde_json::from_str::<Value>(&answer).unwrap(),
        json!({"refused": {"reason": "pinRequired", "retryAfterSeconds": null}})
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
    assert_eq!(value, oxyos_account_proto::ipc::MAX_MESSAGE);
}

#[test]
fn the_local_sign_in_is_read_and_changed_on_the_session_socket() {
    let (socket, dir) = socket_path("session");
    let served = service(&socket, 4, |message| {
        if message == &json!("localSignIn") {
            vec![json!({"localSignIn": {"pin": "none", "password": true}})]
        } else if message["setPin"]["currentPin"] == "1111" {
            vec![json!({"refused": {"reason": "throttled", "retryAfter": 30}})]
        } else if message.get("setPin").is_some() {
            vec![json!({"localSignIn": {"pin": "set", "password": true}})]
        } else {
            vec![json!({"refused": {"reason": "noPin"}})]
        }
    });
    assert_eq!(
        local_sign_in_at(&socket).unwrap(),
        platform::LocalView {
            pin: oxyos_account_proto::pin::PinState::None,
            password: true
        }
    );
    let set = change_local_at(
        &socket,
        "setPin",
        Some(Secret::new("482916")),
        Some(Secret::new("hunter22")),
        None,
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(&set).unwrap(),
        json!({"localSignIn": {"pin": "set", "password": true}})
    );
    let throttled = change_local_at(
        &socket,
        "setPin",
        Some(Secret::new("1357")),
        None,
        Some(Secret::new("1111")),
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(&throttled).unwrap(),
        json!({"refused": {"reason": "throttled", "retryAfter": 30}})
    );
    let off = change_local_at(
        &socket,
        "passwordOff",
        None,
        Some(Secret::new("hunter22")),
        None,
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(&off).unwrap(),
        json!({"refused": {"reason": "noPin"}})
    );
    // Not a change: nothing is sent.
    assert!(change_local_at(&socket, "removePin", None, None, None).is_err());
    assert!(change_local_at(&socket, "erase", None, None, None).is_err());
    let received = served.join().unwrap();
    assert_eq!(received[0], json!("localSignIn"));
    assert_eq!(
        received[1],
        json!({"setPin": {"pin": "482916", "password": "hunter22"}})
    );
    assert_eq!(
        received[2],
        json!({"setPin": {"pin": "1357", "currentPin": "1111"}})
    );
    assert_eq!(
        received[3],
        json!({"setPasswordSignIn": {"enabled": false, "password": "hunter22"}})
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn settings_keychain_requests_go_rebuilt_and_only_settings_ones() {
    let (socket, dir) = socket_path("session");
    let shapes = [
        json!({"keychainItems": [{"itemId": "i1", "type": "login", "title": "Example",
            "username": "bea@example.com", "domains": ["example.com"], "hasCode": false,
            "modifiedAt": "2026-11-08T10:00:00.000Z"}]}),
        json!({"refused": {"reason": "throttled", "retryAfter": 30}}),
        json!({"refused": {"reason": "wrongPassword", "retryAfter": 30}}),
        // Not the keychain's: a token, a snapshot, AutoFill's credential.
        json!({"token": null}),
        json!({"autofillCredential": {"username": "bea@example.com", "password": "fixture-secret"}}),
    ];
    let count = shapes.len();
    let answers = std::sync::Mutex::new(shapes.into_iter());
    let served = service(&socket, count, move |_| {
        vec![answers.lock().unwrap().next().unwrap()]
    });
    let ask = |request: Value| {
        keychain_at(&socket, &Secret::new(request.to_string()))
            .map(|json| serde_json::from_slice::<Value>(&json).unwrap())
    };
    assert_eq!(
        ask(json!({"keychainList": {}})),
        Ok(
            json!({"keychainItems": [{"itemId": "i1", "type": "login", "title": "Example",
            "username": "bea@example.com", "domains": ["example.com"], "hasCode": false,
            "modifiedAt": "2026-11-08T10:00:00.000Z"}]})
        )
    );
    assert_eq!(
        ask(json!({"keychainVerify": {"password": "fixture-password"}})),
        Ok(json!({"refused": {"reason": "throttled", "retryAfter": 30}}))
    );
    // A wait is said only when throttled.
    assert_eq!(
        ask(json!({"keychainVerify": {"password": "fixture-password"}})),
        Ok(json!({"refused": {"reason": "wrongPassword"}}))
    );
    for _ in 0..2 {
        let failed = ask(json!({"keychainGenerate": {}})).unwrap_err();
        assert!(!failed.to_string().contains("fixture-secret"));
    }
    served.join().unwrap();
    // Never sent: the registrar's requests, any other, and what is no request at all.
    for request in [
        json!({"autofillGenerate": {}}),
        json!({"autofillFill": {"desktopEntryId": "so.oxy.dropsea", "ref": "r", "field": "password"}}),
        json!({"keychainLock": {}}),
        json!({"eraseAll": {"password": "fixture-password"}}),
        json!("snapshot"),
        json!({"keychainVerify": {}}),
    ] {
        let refused = ask(request).unwrap_err();
        assert!(matches!(refused, SessionError::Failed(_)), "{refused:?}");
        assert!(!refused.to_string().contains("fixture-password"));
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_setup_tells_its_steps_and_a_failure_that_kept_the_sign_in_is_retried() {
    let (socket, dir) = socket_path("accountd");
    let served = service(&socket, 1, |message| {
        if message.get("enroll").is_some() {
            vec![
                json!({"progress": "securing"}),
                json!({"progress": "verifying"}),
                json!({"progress": "preparing"}),
                json!({"progress": "signingIn"}),
                json!({"failed": {"reason": "unreachable"}}),
            ]
        } else {
            vec![
                json!({"progress": "preparing"}),
                json!({"progress": "signingIn"}),
                json!({"progress": "finishing"}),
                json!({"enrolled": {"accountId": "a1", "user": "oxy-abc", "uid": 1001,
                "profile": {"id": "a1", "username": "nate", "displayName": null, "avatar": null}}}),
            ]
        }
    });
    let mut enrollment = Enrollment::connect(&socket).unwrap();
    assert!(enrollment.retry().is_err(), "nothing sent yet");
    let told = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    {
        let told = told.clone();
        enrollment.on_progress(Box::new(move |stage| {
            told.borrow_mut()
                .push(stage_name(stage).to_str().unwrap().to_owned())
        }));
    }
    enrollment.set_appearance(Appearance::Dark);
    let failed = enrollment
        .enroll(
            "d",
            &Secret::new("s"),
            "a",
            None,
            Some(&Secret::new("4829")),
        )
        .unwrap()
        .to_owned();
    assert_eq!(
        serde_json::from_str::<Value>(&failed).unwrap(),
        json!({"failed": {"reason": "unreachable"}})
    );
    assert_eq!(
        *told.borrow(),
        ["securing", "verifying", "preparing", "signingIn"]
    );
    let enrolled = enrollment.retry().unwrap().to_owned();
    assert_eq!(
        serde_json::from_str::<Value>(&enrolled).unwrap(),
        json!({"enrolled": {"accountId": "a1", "user": "oxy-abc", "name": "nate"}})
    );
    assert_eq!(told.borrow()[4..], ["preparing", "signingIn", "finishing"]);
    assert!(
        enrollment.retry().is_err(),
        "an ended setup sends nothing more"
    );
    let received = served.join().unwrap();
    assert_eq!(
        received[0],
        json!({"enroll": {"deviceId": "d", "deviceSecret": "s", "accountId": "a", "pin": "4829", "appearance": "dark"}})
    );
    assert_eq!(received[1], json!("retry"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[allow(unsafe_code)]
fn the_setups_c_abi_takes_a_progress_function_an_appearance_and_a_retry() {
    let header: String = include_str!("../../include/oxyos_client.h")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    for declared in [
        "typedef void (*OxyOSAccountEnrollmentProgressFn)(void* context, const char* stage);",
        "int oxyos_account_enrollment_on_progress(OxyOSAccountEnrollment* enrollment, OxyOSAccountEnrollmentProgressFn progress, void* context);",
        "int oxyos_account_enrollment_set_appearance(OxyOSAccountEnrollment* enrollment, const char* appearance);",
        "int64_t oxyos_account_enrollment_retry(OxyOSAccountEnrollment* enrollment, char* out, size_t capacity);",
    ] {
        assert!(header.contains(declared), "{declared} is not in oxyos_client.h");
    }
    let _: unsafe extern "C" fn(
        *mut OxyOSAccountEnrollment,
        OxyOSAccountEnrollmentProgressFn,
        *mut c_void,
    ) -> c_int = oxyos_account_enrollment_on_progress;
    let _: unsafe extern "C" fn(*mut OxyOSAccountEnrollment, *const c_char) -> c_int =
        oxyos_account_enrollment_set_appearance;
    let _: unsafe extern "C" fn(*mut OxyOSAccountEnrollment, *mut c_char, usize) -> i64 =
        oxyos_account_enrollment_retry;
    // Null setups are refused, never read; an appearance that is not one sets nothing.
    // SAFETY: null is the case under test.
    unsafe {
        assert_eq!(
            oxyos_account_enrollment_set_appearance(std::ptr::null_mut(), c"dark".as_ptr()),
            crate::OXYOS_ERROR
        );
        assert_eq!(
            oxyos_account_enrollment_on_progress(std::ptr::null_mut(), None, std::ptr::null_mut()),
            crate::OXYOS_ERROR
        );
    }
}

#[test]
fn a_wait_oxy_asks_of_a_setup_is_told_and_its_answer_waited_for() {
    let (socket, dir) = socket_path("accountd");
    let served = service(&socket, 1, |_| {
        vec![
            json!({"progress": "signingIn"}),
            json!({"waiting": {"retryAfterSeconds": 2}}),
            json!({"progress": "finishing"}),
            json!({"progress": "confirming"}),
            json!({"enrolled": {"accountId": "a1", "user": "oxy-abc", "uid": 1001,
            "profile": {"id": "a1", "username": "nate", "displayName": null, "avatar": null}}}),
        ]
    });
    let mut enrollment = Enrollment::connect(&socket).unwrap();
    let told = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    {
        let stages = told.clone();
        enrollment.on_progress(Box::new(move |stage| {
            stages
                .borrow_mut()
                .push(stage_name(stage).to_str().unwrap().to_owned())
        }));
        let waits = told.clone();
        enrollment.on_waiting(Box::new(move |seconds| {
            waits.borrow_mut().push(format!("waiting {seconds}"))
        }));
    }
    let enrolled = enrollment
        .enroll(
            "d",
            &Secret::new("s"),
            "a",
            None,
            Some(&Secret::new("4829")),
        )
        .unwrap()
        .to_owned();
    assert_eq!(
        serde_json::from_str::<Value>(&enrolled).unwrap(),
        json!({"enrolled": {"accountId": "a1", "user": "oxy-abc", "name": "nate"}})
    );
    assert_eq!(
        *told.borrow(),
        ["signingIn", "waiting 2", "finishing", "confirming"]
    );
    served.join().unwrap();
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[allow(unsafe_code)]
fn the_setups_c_abi_takes_a_waiting_function() {
    let header: String = include_str!("../../include/oxyos_client.h")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    for declared in [
        "typedef void (*OxyOSAccountEnrollmentWaitingFn)(void* context, uint64_t retry_after_seconds);",
        "int oxyos_account_enrollment_on_waiting(OxyOSAccountEnrollment* enrollment, OxyOSAccountEnrollmentWaitingFn waiting, void* context);",
    ] {
        assert!(header.contains(declared), "{declared} is not in oxyos_client.h");
    }
    let _: unsafe extern "C" fn(
        *mut OxyOSAccountEnrollment,
        OxyOSAccountEnrollmentWaitingFn,
        *mut c_void,
    ) -> c_int = oxyos_account_enrollment_on_waiting;
    // SAFETY: null is the case under test.
    unsafe {
        assert_eq!(
            oxyos_account_enrollment_on_waiting(std::ptr::null_mut(), None, std::ptr::null_mut()),
            crate::OXYOS_ERROR
        );
    }
}
