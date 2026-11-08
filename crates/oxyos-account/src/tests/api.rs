//! The HTTP client against a server of the tests' own on this machine, answering as Oxy's API
//! answers (`{data}` for the device mint and `/users/me`, the sign-ins bare, errors as
//! `{error, message}`): the requests it sends and how it reads the answers.

use super::*;
use crate::api::HttpApi;
use std::io::{BufRead, BufReader, Read};
use std::net::TcpListener;
use std::thread::JoinHandle;

/// A request as the server received it.
#[derive(Debug)]
struct Received {
    line: String,
    headers: Vec<(String, String)>,
    body: serde_json::Value,
}

impl Received {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

/// Answers one request with `status`, `headers` and `body`, and hands it back.
fn server(status: u16, headers: &str, body: &str) -> (String, JoinHandle<Received>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let response = format!(
        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
         Connection: close\r\n{headers}\r\n{body}",
        body.len()
    );
    let handle = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let mut headers = Vec::new();
        loop {
            let mut header = String::new();
            reader.read_line(&mut header).unwrap();
            let header = header.trim_end();
            if header.is_empty() {
                break;
            }
            let (key, value) = header.split_once(':').unwrap();
            headers.push((key.trim().to_owned(), value.trim().to_owned()));
        }
        let length = headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case("content-length"))
            .map_or(0, |(_, value)| value.parse().unwrap());
        let mut body = vec![0; length];
        reader.read_exact(&mut body).unwrap();
        (&stream).write_all(response.as_bytes()).unwrap();
        Received {
            line: line.trim_end().to_owned(),
            headers,
            body: if body.is_empty() {
                serde_json::Value::Null
            } else {
                serde_json::from_slice(&body).unwrap()
            },
        }
    });
    (base, handle)
}

fn api(base: &str) -> HttpApi {
    HttpApi::new(base).unwrap()
}

#[test]
fn the_mint_presents_the_credential_pinned_to_the_account() {
    let (base, request) = server(
        200,
        "",
        r#"{"data":{"accessToken":"token-1","expiresAt":"2026-11-06T00:00:00.000Z","nextDeviceSecret":"device-secret-5a1c9e","state":{"deviceId":"device-1","accounts":[],"activeAccountId":null,"revision":3,"updatedAt":1}}}"#,
    );
    let token = api(&base).mint_token(&credential(), &account()).unwrap();
    assert_eq!(token.expose(), "token-1");
    let request = request.join().unwrap();
    assert_eq!(request.line, "POST /session/device/token HTTP/1.1");
    assert_eq!(
        request.body,
        serde_json::json!({"deviceId": DEVICE, "deviceSecret": DEVICE_SECRET, "accountId": ACCOUNT})
    );
    assert_eq!(request.header("content-type"), Some("application/json"));
    // A native client: no page's Origin, no cookie, no bearer.
    for header in ["origin", "cookie", "authorization", "sec-fetch-site"] {
        assert_eq!(request.header(header), None, "{header}");
    }
}

#[test]
fn the_mints_refusals_are_told_apart() {
    for (body, error) in [
        (
            r#"{"error":"invalid_device_secret"}"#,
            ApiError::InvalidDeviceSecret,
        ),
        (
            r#"{"error":"account_not_on_device"}"#,
            ApiError::AccountNotOnDevice,
        ),
    ] {
        let (base, _request) = server(401, "", body);
        assert_eq!(
            api(&base)
                .mint_token(&credential(), &account())
                .unwrap_err(),
            error
        );
    }
    let (base, _request) = server(429, "Retry-After: 42\r\n", r#"{"error":"RATE_LIMITED"}"#);
    assert_eq!(
        api(&base)
            .mint_token(&credential(), &account())
            .unwrap_err(),
        ApiError::Locked {
            retry_after: Some(42)
        }
    );
}

#[test]
fn the_profile_is_read_with_the_token() {
    let (base, request) = server(
        200,
        "",
        r#"{"data":{"id":"0190f6c4-8a1e-7b3c-9d2e-4f5a6b7c8d9e","username":"ada","name":{"first":"Ada","last":"Lovelace"},"avatar":"file-9","languages":[]}}"#,
    );
    let profile = api(&base).profile(&Secret::new("token-1")).unwrap();
    assert_eq!(
        profile,
        Profile {
            id: ACCOUNT.into(),
            username: Some("ada".into()),
            display_name: Some("Ada Lovelace".into()),
            avatar: Some("file-9".into()),
            languages: None,
        }
    );
    let request = request.join().unwrap();
    assert_eq!(request.line, "GET /users/me HTTP/1.1");
    assert_eq!(request.header("authorization"), Some("Bearer token-1"));
}

#[test]
fn the_profile_keeps_the_accounts_languages_primary_first() {
    let profile = |languages: &str| {
        let (base, request) = server(
            200,
            "",
            &format!(
                r#"{{"data":{{"id":"0190f6c4-8a1e-7b3c-9d2e-4f5a6b7c8d9e","username":"ada"{languages}}}}}"#
            ),
        );
        let profile = api(&base).profile(&Secret::new("token-1")).unwrap();
        request.join().unwrap();
        profile.languages
    };
    let both = Some(vec!["es-ES".to_owned(), "en-GB".to_owned()]);
    assert_eq!(profile(r#","languages":["es-ES","en-GB"]"#), both);
    // Only its non-empty strings.
    assert_eq!(
        profile(r#","languages":["es-ES",""," ",7,null,"en-GB"]"#),
        both
    );
    // None when it names none, or not as a list.
    assert_eq!(profile(""), None);
    assert_eq!(profile(r#","languages":null"#), None);
    assert_eq!(profile(r#","languages":[""]"#), None);
    assert_eq!(profile(r#","languages":"es-ES""#), None);
}

#[test]
fn the_password_is_checked_on_the_same_device_naming_the_platform() {
    let (base, request) = server(
        200,
        "",
        r#"{"sessionId":"s-1","deviceId":"device-1","expiresAt":"2026-11-06T00:00:00.000Z","accessToken":"token-2","deviceSecret":"platform-secret-1","user":{"id":"0190f6c4-8a1e-7b3c-9d2e-4f5a6b7c8d9e","username":"ada"}}"#,
    );
    let step = api(&base)
        .sign_in_password(
            USERNAME,
            &Secret::new(PASSWORD),
            &credential(),
            Some(&platform_client()),
        )
        .unwrap();
    let SignInStep::SignedIn(signed_in) = step else {
        panic!("signed in: {step:?}");
    };
    // The holder secret it issued is the platform credential: kept.
    assert_eq!(
        signed_in,
        SignedIn {
            user_id: ACCOUNT.into(),
            device_id: DEVICE.into(),
            device_secret: Some(Secret::new("platform-secret-1")),
        }
    );
    let request = request.join().unwrap();
    assert_eq!(request.line, "POST /auth/signin/password HTTP/1.1");
    assert_eq!(
        request.body,
        serde_json::json!({
            "identifier": USERNAME,
            "password": PASSWORD,
            "device": {"deviceId": DEVICE, "deviceSecret": DEVICE_SECRET},
            "platformClient": {"platform": "oxyos", "clientId": CLIENT_ID, "confirms": true},
        })
    );
    // A native client: no web Origin, which would have the platform client refused.
    assert_eq!(request.header("origin"), None);
}

#[test]
fn a_sign_in_without_the_platform_names_none() {
    let (base, request) = server(
        200,
        "",
        r#"{"sessionId":"s-1","deviceId":"device-1","user":{"id":"0190f6c4-8a1e-7b3c-9d2e-4f5a6b7c8d9e"}}"#,
    );
    let step = api(&base)
        .sign_in_password(USERNAME, &Secret::new(PASSWORD), &credential(), None)
        .unwrap();
    assert!(matches!(
        step,
        SignInStep::SignedIn(SignedIn {
            device_secret: None,
            ..
        })
    ));
    assert!(request.join().unwrap().body.get("platformClient").is_none());
}

#[test]
fn an_application_token_is_minted_with_the_attestation() {
    for (attestation, body) in [
        (
            Attestation::Entry("so.oxy.dropsea".into()),
            serde_json::json!({"platform": "oxyos", "desktopEntryId": "so.oxy.dropsea"}),
        ),
        (
            Attestation::OwnPlatform,
            serde_json::json!({"platform": "oxyos", "self": true}),
        ),
    ] {
        // In `sendSuccess`'s envelope or bare: either is read.
        for answer in [
            r#"{"data":{"accessToken":"app-1","expiresAt":"2026-11-06T12:05:00.000Z","applicationId":"app","accountId":"0190f6c4-8a1e-7b3c-9d2e-4f5a6b7c8d9e"}}"#,
            r#"{"accessToken":"app-1","expiresAt":"2026-11-06T12:05:00.000Z","applicationId":"app","accountId":"0190f6c4-8a1e-7b3c-9d2e-4f5a6b7c8d9e"}"#,
        ] {
            let (base, request) = server(200, "", answer);
            let token = api(&base)
                .mint_app_token(&credential(), &account(), &attestation)
                .unwrap();
            assert_eq!(token.access_token.expose(), "app-1");
            assert_eq!(token.expires_at, "2026-11-06T12:05:00.000Z");
            assert_eq!(token.account_id, ACCOUNT);
            let request = request.join().unwrap();
            assert_eq!(request.line, "POST /session/device/app-token HTTP/1.1");
            assert_eq!(
                request.body,
                serde_json::json!({
                    "deviceId": DEVICE,
                    "deviceSecret": DEVICE_SECRET,
                    "accountId": ACCOUNT,
                    "attestation": body,
                })
            );
            for header in ["origin", "cookie", "authorization"] {
                assert_eq!(request.header(header), None, "{header}");
            }
        }
    }
    // Its refusals: the device session gone (revocation, R4), an application not recognised.
    for (status, body, error) in [
        (
            401,
            r#"{"error":"no_active_session"}"#,
            ApiError::AccountNotOnDevice,
        ),
        (
            401,
            r#"{"error":"invalid_device_secret"}"#,
            ApiError::InvalidDeviceSecret,
        ),
        (
            403,
            r#"{"error":"application_not_recognised"}"#,
            ApiError::NotRecognised,
        ),
    ] {
        let (base, _request) = server(status, "", body);
        assert_eq!(
            api(&base)
                .mint_app_token(&credential(), &account(), &Attestation::OwnPlatform)
                .unwrap_err(),
            error
        );
    }
}

#[test]
fn a_device_is_signed_out_with_its_own_token() {
    let (base, request) = server(200, "", r#"{"data":{"deviceId":"device-1","accounts":[]}}"#);
    api(&base).sign_out_device(&Secret::new("token-1")).unwrap();
    let request = request.join().unwrap();
    assert_eq!(request.line, "POST /session/device/signout HTTP/1.1");
    assert_eq!(request.header("authorization"), Some("Bearer token-1"));
    assert_eq!(request.body, serde_json::json!({"all": true}));
}

#[test]
fn an_authenticator_is_a_challenge_answered_with_its_code() {
    let (base, _request) = server(
        200,
        "",
        r#"{"secondFactorRequired":true,"challengeId":"challenge-7f3b","expiresAt":1793000000000}"#,
    );
    let step = api(&base)
        .sign_in_password(
            USERNAME,
            &Secret::new(PASSWORD),
            &credential(),
            Some(&platform_client()),
        )
        .unwrap();
    let SignInStep::SecondFactor { challenge } = step else {
        panic!("a challenge: {step:?}");
    };
    assert_eq!(challenge.expose(), CHALLENGE);

    let (base, request) = server(
        200,
        "",
        r#"{"sessionId":"s-2","deviceId":"device-1","expiresAt":"x","user":{"id":"0190f6c4-8a1e-7b3c-9d2e-4f5a6b7c8d9e"}}"#,
    );
    let signed_in = api(&base)
        .second_factor(
            &challenge,
            &Secret::new(CODE),
            &credential(),
            Some(&platform_client()),
        )
        .unwrap();
    assert_eq!(signed_in.user_id, ACCOUNT);
    let request = request.join().unwrap();
    assert_eq!(request.line, "POST /auth/signin/second-factor HTTP/1.1");
    assert_eq!(
        request.body,
        serde_json::json!({
            "challengeId": CHALLENGE,
            "code": CODE,
            "device": {"deviceId": DEVICE, "deviceSecret": DEVICE_SECRET},
            "platformClient": {"platform": "oxyos", "clientId": CLIENT_ID, "confirms": true},
        })
    );
}

#[test]
fn the_sign_ins_refusals_are_told_apart() {
    for (status, headers, body, error) in [
        (
            401,
            "",
            r#"{"error":"SIGNIN_INVALID_CREDENTIALS","message":"That username, email or password is not right."}"#,
            ApiError::InvalidCredentials,
        ),
        (
            401,
            "",
            r#"{"error":"SECOND_FACTOR_INVALID","message":"x"}"#,
            ApiError::SecondFactorInvalid,
        ),
        (
            429,
            "Retry-After: 900\r\n",
            r#"{"error":"SIGNIN_LOCKED","message":"x","details":{"retryAfterSeconds":900}}"#,
            ApiError::Locked {
                retry_after: Some(900),
            },
        ),
        (
            403,
            "",
            r#"{"error":"SIGNIN_ORIGIN_NOT_ALLOWED","message":"x"}"#,
            ApiError::Refused {
                status: 403,
                code: Some("SIGNIN_ORIGIN_NOT_ALLOWED".into()),
            },
        ),
        (
            500,
            "",
            "not json",
            ApiError::Refused {
                status: 500,
                code: None,
            },
        ),
    ] {
        let (base, _request) = server(status, headers, body);
        assert_eq!(
            api(&base)
                .sign_in_password(USERNAME, &Secret::new(PASSWORD), &credential(), None)
                .unwrap_err(),
            error
        );
    }
}

#[test]
fn an_answer_outside_the_contract_is_no_sign_in() {
    let (base, _request) = server(200, "", r#"{"sessionId":"s-1"}"#);
    assert!(matches!(
        api(&base).sign_in_password(USERNAME, &Secret::new(PASSWORD), &credential(), None),
        Err(ApiError::Unexpected(_))
    ));
}

#[test]
fn an_unreachable_api_is_offline() {
    // A port bound and never listened on: the connection is refused, as with no network. It stays
    // bound for the test, so no other test's server can take it meanwhile (a port bound and let go
    // could be).
    let socket = rustix::net::socket(
        rustix::net::AddressFamily::INET,
        rustix::net::SocketType::STREAM,
        None,
    )
    .unwrap();
    rustix::net::bind(
        &socket,
        &std::net::SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, 0),
    )
    .unwrap();
    let port = std::net::SocketAddrV4::try_from(rustix::net::getsockname(&socket).unwrap())
        .unwrap()
        .port();
    let api = api(&format!("http://127.0.0.1:{port}"));
    assert!(matches!(
        api.mint_token(&credential(), &account()),
        Err(ApiError::Offline(_))
    ));
}

#[test]
fn oxys_api_is_reached_over_https_only() {
    assert!(HttpApi::new("https://api.oxy.so").is_ok());
    assert!(HttpApi::new("http://127.0.0.1:3001").is_ok());
    assert!(HttpApi::new("http://[::1]:3001").is_ok());
    assert!(HttpApi::new("http://localhost:3001").is_ok());
    for base in [
        "http://api.oxy.so",
        "http://192.168.1.2:3001",
        "ftp://api.oxy.so",
        "https://api.oxy.so/?x=1",
        "https://user@api.oxy.so",
        "api.oxy.so",
    ] {
        assert!(HttpApi::new(base).is_err(), "{base}");
    }
}

#[test]
fn a_qr_sign_in_is_redeemed_by_its_device_proof_alone_naming_the_platform() {
    let (base, request) = server(
        200,
        "",
        r#"{"sessionId":"s-1","deviceId":"device-1","expiresAt":"2026-11-06T00:00:00.000Z","accessToken":"token-2","deviceSecret":"platform-secret-1","user":{"id":"0190f6c4-8a1e-7b3c-9d2e-4f5a6b7c8d9e","username":"ada"}}"#,
    );
    let key = RedemptionKey::new().unwrap();
    let signed_in = api(&base)
        .platform_device_sign_in(&credential(), &platform_client(), &key)
        .unwrap();
    assert_eq!(
        signed_in,
        SignedIn {
            user_id: ACCOUNT.into(),
            device_id: DEVICE.into(),
            device_secret: Some(Secret::new("platform-secret-1")),
        }
    );
    let request = request.join().unwrap();
    assert_eq!(request.line, "POST /session/device/platform HTTP/1.1");
    // Exactly the contract's keys: the proof, the platform and the redemption's key (R7), no
    // password, no identifier.
    assert_eq!(
        request.body,
        serde_json::json!({
            "device": {"deviceId": DEVICE, "deviceSecret": DEVICE_SECRET},
            "platformClient": {"platform": "oxyos", "clientId": CLIENT_ID, "confirms": true},
            "redemptionKey": key.expose(),
        })
    );
    // 256 bits, as 64 hexadecimal digits: within the contract's 43 to 128 URL-safe characters.
    assert_eq!(key.expose().len(), 64);
    assert!(key.expose().bytes().all(|b| b.is_ascii_hexdigit()));
    assert_ne!(key.expose(), RedemptionKey::new().unwrap().expose());
    assert_eq!(format!("{key:?}"), "RedemptionKey(…)");
    assert_eq!(request.header("origin"), None);
}

#[test]
fn a_proof_that_cannot_be_redeemed_is_one_refusal_and_a_limit_says_when() {
    let (base, request) = server(
        401,
        "",
        r#"{"error":"invalid_grant","message":"Invalid grant"}"#,
    );
    let key = RedemptionKey::new().unwrap();
    assert_eq!(
        api(&base).platform_device_sign_in(&credential(), &platform_client(), &key),
        Err(ApiError::InvalidDeviceSecret)
    );
    request.join().unwrap();
    let (base, request) = server(429, "Retry-After: 120\r\n", r#"{"error":"rate_limited"}"#);
    assert_eq!(
        api(&base).platform_device_sign_in(&credential(), &platform_client(), &key),
        Err(ApiError::Locked {
            retry_after: Some(120)
        })
    );
    request.join().unwrap();
}

#[test]
fn a_platform_credential_is_confirmed_by_itself_and_one_refusal_says_it_cannot_be() {
    let (base, request) = server(200, "", r#"{"confirmed":true}"#);
    api(&base)
        .confirm_platform_credential(&credential())
        .unwrap();
    let request = request.join().unwrap();
    assert_eq!(
        request.line,
        "POST /session/device/platform/confirm HTTP/1.1"
    );
    // The credential proves itself: its device and secret, no bearer.
    assert_eq!(
        request.body,
        serde_json::json!({"deviceId": DEVICE, "deviceSecret": DEVICE_SECRET})
    );
    assert_eq!(request.header("authorization"), None);
    let (base, request) = server(
        401,
        "",
        r#"{"error":"invalid_grant","message":"Invalid grant"}"#,
    );
    assert_eq!(
        api(&base).confirm_platform_credential(&credential()),
        Err(ApiError::InvalidDeviceSecret)
    );
    request.join().unwrap();
    let (base, request) = server(503, "", r#"{"error":"unavailable"}"#);
    assert!(matches!(
        api(&base).confirm_platform_credential(&credential()),
        Err(ApiError::Refused { status: 503, .. })
    ));
    request.join().unwrap();
}
