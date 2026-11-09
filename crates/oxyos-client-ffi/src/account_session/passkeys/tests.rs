//! WebAuthn's JSON shapes to the service's ceremonies and back, against a played service.

use super::*;
use oxyos_account::ipc::Listener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

fn socket_path() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "oxyos-ffi-passkeys-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("session")
}

/// A service answering one connection's one message with `answer`; the message it received.
fn service(socket: &Path, answer: Value) -> std::thread::JoinHandle<Value> {
    let listener = Listener::bind(socket).unwrap();
    std::thread::spawn(move || {
        let connection = listener.accept().unwrap();
        let message = connection.receive::<Value>().unwrap().unwrap().unwrap();
        connection.send(&answer).unwrap();
        message
    })
}

fn creation() -> Value {
    json!({
        "rp": {"id": "github.com", "name": "GitHub"},
        "user": {"id": "YWRh", "name": "ada", "displayName": "Ada Lovelace"},
        "challenge": "Y2hhbGxlbmdl",
        "pubKeyCredParams": [{"type": "public-key", "alg": -7}, {"type": "public-key", "alg": -257}],
        "timeout": 60000,
        "excludeCredentials": [{"type": "public-key", "id": "AAAA", "transports": ["internal"]}],
        "authenticatorSelection": {"residentKey": "required", "userVerification": "required"},
        "attestation": "none",
        "extensions": {"credProps": true},
    })
}

fn decoded(text: &str) -> String {
    String::from_utf8(URL_SAFE_NO_PAD.decode(text).unwrap()).unwrap()
}

#[test]
fn a_creation_is_the_services_ceremony_over_the_client_data_made_here() {
    let socket = socket_path();
    let served = service(
        &socket,
        json!({"passkeyCreated": {"credentialId": "Q1JFRA", "attestationObject": "QVRU",
            "authenticatorData": "QVVUSA", "publicKey": "U1BLSQ", "publicKeyAlgorithm": -7,
            "transports": ["internal"]}}),
    );
    let Outcome::Credential(credential) = create_at(&socket, &creation()).unwrap() else {
        panic!("no credential");
    };
    let asked = served.join().unwrap();
    let client_data = decoded(credential["response"]["clientDataJSON"].as_str().unwrap());
    // WebAuthn's serialization, the origin the relying party's.
    assert_eq!(
        client_data,
        r#"{"type":"webauthn.create","challenge":"Y2hhbGxlbmdl","origin":"https://github.com","crossOrigin":false}"#
    );
    assert_eq!(
        asked,
        json!({"passkeyCreate": {"rpId": "github.com", "rpName": "GitHub",
            "user": {"id": "YWRh", "name": "ada", "displayName": "Ada Lovelace"},
            "clientDataHash": URL_SAFE_NO_PAD.encode(ring::digest::digest(&ring::digest::SHA256, client_data.as_bytes())),
            "excludeCredentials": ["AAAA"], "algorithms": [-7, -257]}})
    );
    assert_eq!(
        credential,
        json!({"id": "Q1JFRA", "rawId": "Q1JFRA", "type": "public-key",
            "authenticatorAttachment": "platform",
            "response": {"clientDataJSON": credential["response"]["clientDataJSON"],
                "attestationObject": "QVRU", "authenticatorData": "QVVUSA", "publicKey": "U1BLSQ",
                "publicKeyAlgorithm": -7, "transports": ["internal"]},
            "clientExtensionResults": {}})
    );
}

#[test]
fn an_assertion_is_the_services_over_the_client_data_made_here() {
    let socket = socket_path();
    let served = service(
        &socket,
        json!({"passkeyAssertion": {"credentialId": "Q1JFRA", "authenticatorData": "QVVUSA",
            "signature": "U0lH", "userHandle": "YWRh"}}),
    );
    let options = json!({"challenge": "Z2V0", "rpId": "github.com", "userVerification": "required",
        "allowCredentials": [{"type": "public-key", "id": "Q1JFRA"}, {"type": "other", "id": "!"}]});
    let Outcome::Credential(credential) = get_at(&socket, &options).unwrap() else {
        panic!("no assertion");
    };
    let asked = served.join().unwrap();
    let client_data = decoded(credential["response"]["clientDataJSON"].as_str().unwrap());
    assert_eq!(
        client_data,
        r#"{"type":"webauthn.get","challenge":"Z2V0","origin":"https://github.com","crossOrigin":false}"#
    );
    assert_eq!(
        asked,
        json!({"passkeyGet": {"rpId": "github.com",
            "clientDataHash": URL_SAFE_NO_PAD.encode(ring::digest::digest(&ring::digest::SHA256, client_data.as_bytes())),
            "allowCredentials": ["Q1JFRA"]}})
    );
    assert_eq!(credential["response"]["signature"], "U0lH");
    assert_eq!(credential["response"]["userHandle"], "YWRh");
    assert_eq!(credential["authenticatorAttachment"], "platform");
}

#[test]
fn refusals_are_webauthns_errors() {
    for (reason, name) in [
        ("notAllowed", "NotAllowedError"),
        ("locked", "NotAllowedError"),
        ("notPermitted", "NotAllowedError"),
        ("invalidState", "InvalidStateError"),
        ("notSupported", "NotSupportedError"),
        ("invalidRequest", "TypeError"),
        ("offline", "UnknownError"),
        ("failed", "UnknownError"),
    ] {
        let socket = socket_path();
        let served = service(&socket, json!({"refused": {"reason": reason}}));
        assert_eq!(
            get_at(&socket, &json!({"challenge": "Z2V0", "rpId": "github.com"}))
                .unwrap()
                .to_json(),
            json!({"error": {"name": name, "reason": reason}})
        );
        served.join().unwrap();
    }
}

#[test]
fn what_is_not_webauthns_shape_is_a_type_error_and_never_asked() {
    // Nothing listens: a request that reached the service would fail otherwise.
    let socket = socket_path();
    let without = |key: &str| {
        let mut options = creation();
        options.as_object_mut().unwrap().remove(key);
        options
    };
    for options in [
        without("rp"),
        without("user"),
        without("challenge"),
        without("pubKeyCredParams"),
        json!({"rp": {"name": "GitHub"}, "user": creation()["user"], "challenge": "Y2g", "pubKeyCredParams": []}),
        {
            let mut options = creation();
            options["challenge"] = json!("not base64url!");
            options
        },
        {
            let mut options = creation();
            options["user"]["id"] = json!("a=");
            options
        },
        {
            let mut options = creation();
            options["excludeCredentials"] = json!([{"type": "public-key", "id": "+/"}]);
            options
        },
        json!([]),
    ] {
        let outcome = create_at(&socket, &options).unwrap();
        assert!(
            matches!(
                outcome,
                Outcome::Error {
                    name: "TypeError",
                    ..
                }
            ),
            "{options}: {outcome:?}"
        );
    }
    // Only algorithms of another type: none OxyOS makes.
    let mut options = creation();
    options["pubKeyCredParams"] = json!([{"type": "other", "alg": -7}]);
    assert!(matches!(
        create_at(&socket, &options).unwrap(),
        Outcome::Error {
            name: "NotSupportedError",
            ..
        }
    ));
    // A security key alone: not this platform authenticator's.
    let mut options = creation();
    options["authenticatorSelection"]["authenticatorAttachment"] = json!("cross-platform");
    assert!(matches!(
        create_at(&socket, &options).unwrap(),
        Outcome::Error {
            name: "NotAllowedError",
            ..
        }
    ));
    for options in [json!({"challenge": "Z2V0"}), json!({"rpId": "github.com"})] {
        assert!(matches!(
            get_at(&socket, &options).unwrap(),
            Outcome::Error {
                name: "TypeError",
                ..
            }
        ));
    }
    // A well-formed request with nothing listening: the service could not be asked.
    assert!(get_at(&socket, &json!({"challenge": "Z2V0", "rpId": "github.com"})).is_err());
}

#[test]
fn no_algorithm_asked_is_es256_and_rs256() {
    let socket = socket_path();
    let served = service(&socket, json!({"refused": {"reason": "notAllowed"}}));
    let mut options = creation();
    options["pubKeyCredParams"] = json!([]);
    create_at(&socket, &options).unwrap();
    assert_eq!(
        served.join().unwrap()["passkeyCreate"]["algorithms"],
        json!([-7, -257])
    );
}

#[test]
fn an_answer_that_is_not_a_credential_is_none() {
    let socket = socket_path();
    let served = service(
        &socket,
        json!({"passkeyCreated": {"credentialId": "", "attestationObject": "QVRU",
            "authenticatorData": "QVVUSA", "publicKey": "U1BLSQ", "publicKeyAlgorithm": -7,
            "transports": []}}),
    );
    assert!(create_at(&socket, &creation()).is_err());
    served.join().unwrap();
}

#[test]
#[allow(unsafe_code)]
fn the_c_abi_answers_json() {
    let mut out = vec![0 as std::ffi::c_char; 4096];
    let ceremony = std::ffi::CString::new("sign").unwrap();
    let options = std::ffi::CString::new("{}").unwrap();
    // SAFETY: both strings are NUL-terminated and `out` holds `out.len()` bytes.
    let length = unsafe {
        oxyos_account_session_passkey(
            ceremony.as_ptr(),
            options.as_ptr(),
            out.as_mut_ptr(),
            out.len(),
        )
    };
    assert!(length > 0);
    // SAFETY: the library wrote a NUL-terminated string.
    let text = unsafe { std::ffi::CStr::from_ptr(out.as_ptr()) }
        .to_str()
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(text).unwrap(),
        json!({"error": {"name": "TypeError", "reason": "a ceremony is create or get"}})
    );
}
