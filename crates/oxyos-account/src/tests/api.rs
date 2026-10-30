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
        }
    );
    let request = request.join().unwrap();
    assert_eq!(request.line, "GET /users/me HTTP/1.1");
    assert_eq!(request.header("authorization"), Some("Bearer token-1"));
}

#[test]
fn the_password_is_checked_on_the_same_device() {
    let (base, request) = server(
        200,
        "",
        r#"{"sessionId":"s-1","deviceId":"device-1","expiresAt":"2026-11-06T00:00:00.000Z","accessToken":"token-2","deviceSecret":"device-secret-new","user":{"id":"0190f6c4-8a1e-7b3c-9d2e-4f5a6b7c8d9e","username":"ada"}}"#,
    );
    let step = api(&base)
        .sign_in_password(USERNAME, &Secret::new(PASSWORD), &credential())
        .unwrap();
    let SignInStep::SignedIn(signed_in) = step else {
        panic!("signed in: {step:?}");
    };
    assert_eq!(
        signed_in,
        SignedIn {
            user_id: ACCOUNT.into(),
            device_id: DEVICE.into()
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
        })
    );
    assert_eq!(request.header("origin"), None);
}

#[test]
fn an_authenticator_is_a_challenge_answered_with_its_code() {
    let (base, _request) = server(
        200,
        "",
        r#"{"secondFactorRequired":true,"challengeId":"challenge-7f3b","expiresAt":1793000000000}"#,
    );
    let step = api(&base)
        .sign_in_password(USERNAME, &Secret::new(PASSWORD), &credential())
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
        .second_factor(&challenge, &Secret::new(CODE), &credential())
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
                .sign_in_password(USERNAME, &Secret::new(PASSWORD), &credential())
                .unwrap_err(),
            error
        );
    }
}

#[test]
fn an_answer_outside_the_contract_is_no_sign_in() {
    let (base, _request) = server(200, "", r#"{"sessionId":"s-1"}"#);
    assert!(matches!(
        api(&base).sign_in_password(USERNAME, &Secret::new(PASSWORD), &credential()),
        Err(ApiError::Unexpected(_))
    ));
}

#[test]
fn an_unreachable_api_is_offline() {
    // A port nothing listens on: the connection is refused, as with no network.
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
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
