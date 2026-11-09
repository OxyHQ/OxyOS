//! Passkeys (ADR 0050 step 5): registered applications creating and using passkeys on the session
//! socket, each verified on the session's sheet (the test standing for the session and the
//! person), and every registration and assertion read and verified as a relying party would, by
//! ring's ECDSA and ciborium's CBOR, not the service's own code.

use super::*;
use crate::ipc::Connection;
use crate::keychain::passkey::*;
use crate::keychain::service::RelyingParties;
use ciborium::value::Value as Cbor;
use oxyos_keychain::wire::{b64, unb64};
use rustix::event::{poll, PollFd, PollFlags};
use sha2::{Digest, Sha256};

const MENTION: &str = "so.oxy.mention";
const ASTRO: &str = "so.oxy.astro";
const DROPSEA: &str = "so.oxy.dropsea";
/// OxyOS's AAGUID, as ADR 0050 documents it.
const AAGUID: &str = "7c6ab314ebcd3fa151095ada4bb93e0b";
const SPKI_PREFIX: &str = "3059301306072a8648ce3d020106082a8648ce3d030107034200";

fn hash(client_data: &str) -> String {
    b64(&Sha256::digest(client_data.as_bytes()))
}

fn create_request(rp_id: &str, handle: &[u8], exclude: &[String]) -> Value {
    json!({"passkeyCreate": {
        "rpId": rp_id,
        "rpName": "Relying Party",
        "user": {"id": b64(handle), "name": "ada@example.com", "displayName": "Ada"},
        "clientDataHash": hash(&format!("create {rp_id}")),
        "excludeCredentials": exclude,
        "algorithms": [-8, -7],
    }})
}

fn get_request(rp_id: &str, client_data: &str, allow: &[String]) -> Value {
    json!({"passkeyGet": {
        "rpId": rp_id,
        "clientDataHash": hash(client_data),
        "allowCredentials": allow,
    }})
}

fn not_allowed() -> Value {
    json!({"refused": {"reason": "notAllowed"}})
}

/// A registration, read as a relying party reads it: the public key's uncompressed point.
fn registered(created: &Value, rp_id: &str) -> Vec<u8> {
    let created = &created["passkeyCreated"];
    assert_eq!(created["publicKeyAlgorithm"], -7, "{created}");
    assert_eq!(created["transports"], json!(["internal"]));
    let decode = |field: &str| unb64(created[field].as_str().unwrap(), "test").unwrap();
    let object: Cbor = ciborium::from_reader(decode("attestationObject").as_slice()).unwrap();
    let entry = |key: &str| {
        object
            .as_map()
            .unwrap()
            .iter()
            .find(|(k, _)| k.as_text() == Some(key))
            .map(|(_, v)| v.clone())
            .unwrap()
    };
    assert_eq!(entry("fmt").as_text(), Some("none"));
    assert!(entry("attStmt").as_map().unwrap().is_empty());
    let data = entry("authData").into_bytes().unwrap();
    assert_eq!(data, decode("authenticatorData"));
    assert_eq!(&data[..32], Sha256::digest(rp_id.as_bytes()).as_slice());
    assert_eq!(data[32], 0x5d, "UP, UV, BE, BS and AT");
    assert_eq!(&data[33..37], &[0, 0, 0, 0]);
    assert_eq!(hex(&data[37..53]), AAGUID);
    assert_eq!(&data[53..55], &[0, 16]);
    assert_eq!(&data[55..71], decode("credentialId").as_slice());
    let key: Cbor = ciborium::from_reader(&data[71..]).unwrap();
    let label = |n: i64| {
        key.as_map()
            .unwrap()
            .iter()
            .find(|(k, _)| k.as_integer() == Some(n.into()))
            .map(|(_, v)| v.clone())
            .unwrap()
    };
    let point = [
        vec![4u8],
        label(-2).into_bytes().unwrap(),
        label(-3).into_bytes().unwrap(),
    ]
    .concat();
    // getPublicKey() is the same point.
    assert_eq!(
        hex(&decode("publicKey")),
        format!("{SPKI_PREFIX}{}", hex(&point))
    );
    point
}

/// An assertion verified by ring with the registration's point, over `client_data`'s hash.
fn verifies(assertion: &Value, point: &[u8], rp_id: &str, client_data: &str) -> bool {
    let assertion = &assertion["passkeyAssertion"];
    let decode = |field: &str| unb64(assertion[field].as_str().unwrap(), "test").unwrap();
    let data = decode("authenticatorData");
    assert_eq!(data.len(), 37);
    assert_eq!(&data[..32], Sha256::digest(rp_id.as_bytes()).as_slice());
    assert_eq!(data[32], 0x1d, "UP, UV, BE and BS");
    let signed = [data, Sha256::digest(client_data.as_bytes()).to_vec()].concat();
    ring::signature::UnparsedPublicKey::new(&ring::signature::ECDSA_P256_SHA256_ASN1, point)
        .verify(&signed, &decode("signature"))
        .is_ok()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The session's sheet as the test drives it: the prompts it is told, and the person's answers.
trait Sheet {
    fn connection(&self) -> &Connection;

    /// The next prompt, within ten seconds.
    fn prompt(&self) -> Prompt {
        let mut fds = [PollFd::new(self.connection(), PollFlags::IN)];
        let ready = poll(
            &mut fds,
            Some(&rustix::time::Timespec {
                tv_sec: 10,
                tv_nsec: 0,
            }),
        )
        .unwrap();
        assert_eq!(ready, 1, "no prompt came");
        self.connection()
            .receive::<Prompt>()
            .unwrap()
            .unwrap()
            .unwrap()
    }

    /// No prompt within a moment.
    fn quiet(&self) {
        let mut fds = [PollFd::new(self.connection(), PollFlags::IN)];
        let ready = poll(
            &mut fds,
            Some(&rustix::time::Timespec {
                tv_sec: 0,
                tv_nsec: 300_000_000,
            }),
        )
        .unwrap();
        assert_eq!(ready, 0, "a prompt was shown");
    }

    fn confirm(&self, id: &str, choice: Option<usize>, password: Option<&str>) {
        self.connection()
            .send(&Request::PasskeyConfirm {
                id: id.into(),
                choice,
                password: password.map(Secret::new),
            })
            .unwrap();
    }

    fn cancel(&self, id: &str) {
        self.connection()
            .send(&Request::PasskeyCancel { id: id.into() })
            .unwrap();
    }

    /// The sheet asked to confirm, checked to be of `kind` for `rp_id`: its id.
    fn shown(&self, kind: PromptKind, rp_id: &str) -> ConfirmPasskey {
        match self.prompt() {
            Prompt::ConfirmPasskey(confirm) => {
                assert_eq!(confirm.kind, kind);
                assert_eq!(confirm.rp_id, rp_id);
                confirm
            }
            other => panic!("not a sheet: {other:?}"),
        }
    }

    fn done(&self, id: &str) {
        assert_eq!(self.prompt(), Prompt::PasskeyDone { id: id.into() });
    }
}

struct Pair(Connection);

impl Sheet for Pair {
    fn connection(&self) -> &Connection {
        &self.0
    }
}

/// The browsers file, the system's (here the test's own user's), naming Astro.
fn browsers(dir: &TempDir) -> std::path::PathBuf {
    let etc = dir.path().join("etc");
    std::fs::create_dir(&etc).unwrap();
    let path = etc.join("browsers");
    std::fs::write(&path, "# The system's browsers.\nso.oxy.astro\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&etc, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    path
}

fn parties(dir: &TempDir) -> RelyingParties {
    browsers(dir);
    parties_at(dir)
}

/// The test's rule over a browsers file already written.
fn parties_at(dir: &TempDir) -> RelyingParties {
    RelyingParties {
        browsers: dir.path().join("etc").join("browsers"),
        owner: sockets::me(),
        web_origins: Box::new(|id| match id {
            MENTION => vec![
                "https://mention.earth".into(),
                "https://app.mention.earth".into(),
            ],
            _ => Vec::new(),
        }),
    }
}

#[test]
fn which_relying_parties_an_application_may_use() {
    let dir = TempDir::new("passkeys-parties");
    let parties = parties(&dir);
    assert!(parties.allows(MENTION, "mention.earth"));
    // A registrable domain suffix of an origin's host, never a public suffix.
    assert!(parties.allows(MENTION, "app.mention.earth"));
    assert!(!parties.allows(MENTION, "earth"));
    assert!(!parties.allows(MENTION, "other.mention.earth"));
    assert!(!parties.allows(MENTION, "github.com"));
    assert!(!parties.allows(MENTION, "Mention.earth"));
    assert!(!parties.allows(DROPSEA, "mention.earth"));
    // The browser, named by the system's file: any relying party.
    assert!(parties.is_browser(ASTRO));
    assert!(parties.allows(ASTRO, "github.com"));
    assert!(!parties.allows(ASTRO, "not a domain"));
    // Not when the file is anyone else's to write, nor through a link, nor another owner's.
    use std::os::unix::fs::PermissionsExt;
    let path = parties.browsers.clone();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).unwrap();
    assert!(!parties.allows(ASTRO, "github.com"));
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(parties.allows(ASTRO, "github.com"));
    let etc = path.parent().unwrap();
    std::fs::set_permissions(etc, std::fs::Permissions::from_mode(0o777)).unwrap();
    assert!(!parties.is_browser(ASTRO));
    std::fs::set_permissions(etc, std::fs::Permissions::from_mode(0o755)).unwrap();
    let real = etc.join("real");
    std::fs::rename(&path, &real).unwrap();
    std::os::unix::fs::symlink(&real, &path).unwrap();
    assert!(!parties.is_browser(ASTRO));
    std::fs::remove_file(&path).unwrap();
    std::fs::rename(&real, &path).unwrap();
    let other_owner = RelyingParties {
        browsers: path.clone(),
        owner: sockets::me() + 1,
        web_origins: Box::new(|_| Vec::new()),
    };
    assert!(!other_owner.is_browser(ASTRO));
    // The system's own: root's file, and no application's origins yet.
    let system = RelyingParties::system();
    assert_eq!(system.browsers, std::path::Path::new("/etc/oxyos/browsers"));
    assert!(!system.allows(MENTION, "mention.earth"));
}

#[test]
fn on_the_session_socket_applications_create_and_use_passkeys() {
    let (oxy, base) = Oxy::start();
    let socket = sockets::service(FakeApi::default());
    let dir = TempDir::new("passkeys-socket");
    let keychains = KeychainService::new(
        HttpApi::new(&base).unwrap(),
        dir.path().join("keychains"),
        "ThinkPad".into(),
        Box::new(|_, _| Some(Secret::new("device:a"))),
        Memory::BestEffort,
    );
    std::fs::create_dir(dir.path().join("keychains")).unwrap();
    keychains.set_relying_parties(parties(&dir));
    socket.machine.platform.set_keychains(keychains.clone());
    let uid = sockets::me();
    let user = account().user_name().as_str().to_owned();
    let registrar = Registrar::connect(&socket.path).unwrap();
    let register = |app: &Application, id: &str| {
        registrar
            .register(id, false, app.pidfd().as_fd())
            .unwrap()
            .unwrap();
    };

    // Locked: refused `locked`, whatever the application and the relying party, and no sheet.
    let mut early = Application::start(
        &socket,
        json!([{"send": create_request("mention.earth", &[1], &[])}]),
    );
    register(&early, MENTION);
    early.go();
    assert_eq!(early.line(), json!({"refused": {"reason": "locked"}}));
    early.end();
    let mut stranger = Application::start(
        &socket,
        json!([{"send": get_request("mention.earth", "x", &[])}]),
    );
    register(&stranger, DROPSEA);
    stranger.go();
    assert_eq!(stranger.line(), json!({"refused": {"reason": "locked"}}));
    stranger.end();

    // Unlocked by the lock's check; the account's vault made.
    crate::session::check(
        &socket.path,
        Secret::new(PASSWORD),
        crate::platform::Local::Accepted,
    )
    .unwrap();
    sockets::wait_for("the keychain unlocked", || {
        !matches!(
            keychains.settings(uid, &user, Request::KeychainStatus {}),
            Answer::KeychainStatus(Status {
                state: KeychainState::Locked,
                ..
            })
        )
    });
    // Unlocked but no vault (the keychain is off): `notAllowed` before any sheet, as a relying
    // party it may not use.
    {
        let off = PromptsClient::connect(&socket.path).unwrap();
        let off_sheet = Pair(Connection::from_fd(
            rustix::io::fcntl_dupfd_cloexec(&off, 0).unwrap(),
        ));
        let mut early = Application::start(
            &socket,
            json!([{"send": create_request("mention.earth", &[1], &[])}]),
        );
        register(&early, MENTION);
        early.go();
        assert_eq!(early.line(), not_allowed());
        off_sheet.quiet();
        early.end();
    }
    let Answer::KeychainRecoveryKey { recovery_key } =
        keychains.settings(uid, &user, Request::KeychainCreate {})
    else {
        panic!("no recovery key");
    };
    keychains.settings(
        uid,
        &user,
        Request::KeychainConfirmRecoveryKey { recovery_key },
    );

    // No session showing sheets: refused at once.
    let mut alone = Application::start(
        &socket,
        json!([{"send": create_request("mention.earth", &[1], &[])}]),
    );
    register(&alone, MENTION);
    alone.go();
    assert_eq!(alone.line(), not_allowed());
    alone.end();

    // The session's prompts connection, a registrar connection of its own.
    let client = PromptsClient::connect(&socket.path).unwrap();
    // The test reads the prompts on the connection's own descriptor.
    let sheet = Pair(Connection::from_fd(
        rustix::io::fcntl_dupfd_cloexec(&client, 0).unwrap(),
    ));

    // Mention creates a passkey for its own domain: the sheet, a wrong password (asked again),
    // then the right one.
    let handle = [0xadu8; 32];
    let mut mention = Application::start(
        &socket,
        json!([
            {"send": create_request("mention.earth", &handle, &[])},
            {"send": get_request("mention.earth", "get one", &[])},
            "wait",
            {"send": get_request("mention.earth", "cancelled", &[])},
            {"send": get_request("mention.earth", "no password", &[])},
            "wait",
            {"send": get_request("github.com", "not mine", &[])},
            {"send": create_request("github.com", &handle, &[])},
        ]),
    );
    register(&mention, MENTION);
    mention.go();
    let shown = sheet.shown(PromptKind::Create, "mention.earth");
    assert_eq!(shown.desktop_entry_id, MENTION);
    assert_eq!(
        shown.user_name.as_ref().unwrap().expose(),
        "ada@example.com"
    );
    assert!(shown.pid > 0);
    sheet.confirm(&shown.id, None, Some("not the password"));
    assert_eq!(
        sheet.prompt(),
        Prompt::PasskeyRetry {
            id: shown.id.clone(),
            reason: RetryReason::WrongPassword,
            retry_after: None
        }
    );
    sheet.confirm(&shown.id, None, Some(PASSWORD));
    sheet.done(&shown.id);
    let created = mention.line();
    let point = registered(&created, "mention.earth");
    let credential = created["passkeyCreated"]["credentialId"]
        .as_str()
        .unwrap()
        .to_owned();
    // Oxy holds it: the passkey is an item of the vault, synced.
    assert!(oxy
        .requests()
        .iter()
        .any(|r| r.starts_with("PUT /keychain/items/")));

    // Mention uses it: the sheet, the password, an assertion ring verifies.
    let shown = sheet.shown(PromptKind::Get, "mention.earth");
    assert_eq!(shown.choices.len(), 1);
    assert_eq!(shown.choices[0].user_name.expose(), "ada@example.com");
    assert_eq!(shown.choices[0].display_name.expose(), "Ada");
    sheet.confirm(&shown.id, Some(0), Some(PASSWORD));
    sheet.done(&shown.id);
    let assertion = mention.line();
    assert!(verifies(&assertion, &point, "mention.earth", "get one"));
    assert!(!verifies(
        &assertion,
        &point,
        "mention.earth",
        "another challenge"
    ));
    assert_eq!(
        assertion["passkeyAssertion"]["credentialId"],
        credential.as_str()
    );
    assert_eq!(assertion["passkeyAssertion"]["userHandle"], b64(&handle));
    mention.go();
    // The person cancels: notAllowed.
    let shown = sheet.shown(PromptKind::Get, "mention.earth");
    sheet.cancel(&shown.id);
    sheet.done(&shown.id);
    assert_eq!(mention.line(), not_allowed());
    // No password, no signature: asked again until cancelled.
    let shown = sheet.shown(PromptKind::Get, "mention.earth");
    sheet.confirm(&shown.id, Some(0), None);
    assert!(matches!(
        sheet.prompt(),
        Prompt::PasskeyRetry {
            reason: RetryReason::WrongPassword,
            ..
        }
    ));
    sheet.cancel(&shown.id);
    sheet.done(&shown.id);
    assert_eq!(mention.line(), not_allowed());
    mention.go();
    // A relying party that is not Mention's: refused at once, no sheet, as if it had none.
    assert_eq!(mention.line(), not_allowed());
    assert_eq!(mention.line(), not_allowed());
    sheet.quiet();
    mention.end();

    // Dropsea may use no relying party: the same answer whether the keychain holds passkeys
    // for it or not, and no sheet.
    let mut dropsea = Application::start(
        &socket,
        json!([
            {"send": get_request("mention.earth", "probe", &[])},
            {"send": get_request("mention.earth", "probe", &[credential.clone()])},
            {"send": get_request("nothing.example", "probe", &[])},
            {"send": create_request("mention.earth", &handle, &[credential.clone()])},
        ]),
    );
    register(&dropsea, DROPSEA);
    dropsea.go();
    for _ in 0..4 {
        assert_eq!(dropsea.line(), not_allowed());
    }
    sheet.quiet();
    dropsea.end();

    // Astro, the system's browser, may use any relying party: Mention's passkey on its page, a
    // new one for github.com; excluded credentials and none at all told on the sheet first.
    let mut astro = Application::start(
        &socket,
        json!([
            {"send": get_request("mention.earth", "page", &[credential.clone()])},
            {"send": create_request("github.com", &[7, 7], &[])},
            {"send": create_request("mention.earth", &handle, &[b64(&[0; 16]), credential.clone()])},
            {"send": get_request("example.org", "none", &[])},
            {"send": json!({"passkeyCreate": {"rpId": "github.com", "rpName": "GitHub",
                "user": {"id": "AQ", "name": "ada", "displayName": "Ada"},
                "clientDataHash": hash("rsa"), "algorithms": [-257]}})},
            {"send": json!({"passkeyGet": {"rpId": "github.com", "clientDataHash": "c2hvcnQ"}})},
            {"send": json!({"passkeyCreate": {"rpId": "example.net", "rpName": "Example",
                "user": {"id": "AQ", "name": "ada\u{202e}moc.live\u{0007}", "displayName": "Ada"},
                "clientDataHash": hash("bidi"), "algorithms": [-7]}})},
        ]),
    );
    register(&astro, ASTRO);
    astro.go();
    let shown = sheet.shown(PromptKind::Get, "mention.earth");
    assert_eq!(shown.desktop_entry_id, ASTRO);
    sheet.confirm(&shown.id, Some(0), Some(PASSWORD));
    sheet.done(&shown.id);
    assert!(verifies(&astro.line(), &point, "mention.earth", "page"));
    let shown = sheet.shown(PromptKind::Create, "github.com");
    sheet.confirm(&shown.id, None, Some(PASSWORD));
    sheet.done(&shown.id);
    registered(&astro.line(), "github.com");
    let shown = sheet.shown(PromptKind::Excluded, "mention.earth");
    sheet.confirm(&shown.id, None, None);
    sheet.done(&shown.id);
    assert_eq!(astro.line(), json!({"refused": {"reason": "invalidState"}}));
    let shown = sheet.shown(PromptKind::None, "example.org");
    assert!(shown.choices.is_empty());
    sheet.confirm(&shown.id, None, None);
    sheet.done(&shown.id);
    assert_eq!(astro.line(), not_allowed());
    assert_eq!(astro.line(), json!({"refused": {"reason": "notSupported"}}));
    assert_eq!(
        astro.line(),
        json!({"refused": {"reason": "invalidRequest"}})
    );
    // A name that would reorder the sheet's text is shown without the marks.
    let shown = sheet.shown(PromptKind::Create, "example.net");
    assert_eq!(shown.user_name.as_ref().unwrap().expose(), "adamoc.live");
    sheet.cancel(&shown.id);
    sheet.done(&shown.id);
    assert_eq!(astro.line(), not_allowed());
    sheet.quiet();
    astro.end();

    // The passkeys are items Settings lists (no private key), and the browser's for no app.
    keychains.settings(
        uid,
        &user,
        Request::KeychainVerify {
            password: Secret::new(PASSWORD),
        },
    );
    let Answer::KeychainItems(items) = keychains.settings(
        uid,
        &user,
        Request::KeychainList {
            query: None,
            kind: Some(ItemKind::Passkey),
        },
    ) else {
        panic!("no list");
    };
    let mut domains: Vec<String> = items
        .iter()
        .map(|item| item.domains[0].expose().to_owned())
        .collect();
    domains.sort();
    assert_eq!(domains, ["github.com", "mention.earth"]);

    // Mention's web origins gone from the registry: the passkey of its own still signs for it,
    // but it may make none for a relying party it holds none of, nor another application use it.
    keychains.set_relying_parties(RelyingParties {
        browsers: dir.path().join("etc").join("browsers"),
        owner: sockets::me(),
        web_origins: Box::new(|_| Vec::new()),
    });
    let mut own = Application::start(
        &socket,
        json!([
            {"send": get_request("mention.earth", "own", &[])},
            {"send": create_request("app.mention.earth", &handle, &[])},
        ]),
    );
    register(&own, MENTION);
    own.go();
    let shown = sheet.shown(PromptKind::Get, "mention.earth");
    sheet.confirm(&shown.id, Some(0), Some(PASSWORD));
    sheet.done(&shown.id);
    assert!(verifies(&own.line(), &point, "mention.earth", "own"));
    assert_eq!(own.line(), not_allowed());
    sheet.quiet();
    own.end();
    keychains.set_relying_parties(parties_at(&dir));

    // An application that goes while its sheet is shown: the sheet goes too.
    let mut leaving = Application::start(
        &socket,
        json!([{"send": get_request("mention.earth", "x", &[])}]),
    );
    register(&leaving, MENTION);
    leaving.go();
    let shown = sheet.shown(PromptKind::Get, "mention.earth");
    leaving.end();
    sheet.done(&shown.id);

    // The session locks while a sheet is shown: it goes, and the application is told `locked`.
    let mut locking = Application::start(
        &socket,
        json!([
            {"send": get_request("mention.earth", "x", &[])},
            {"send": get_request("mention.earth", "y", &[])},
        ]),
    );
    register(&locking, MENTION);
    locking.go();
    let shown = sheet.shown(PromptKind::Get, "mention.earth");
    registrar.keychain_lock().unwrap().unwrap();
    sheet.done(&shown.id);
    assert_eq!(locking.line(), json!({"refused": {"reason": "locked"}}));
    assert_eq!(locking.line(), json!({"refused": {"reason": "locked"}}));
    sheet.quiet();
    locking.end();

    // The prompts connection is the registrar's alone; an application cannot open one.
    let mut pretender = Application::start(
        &socket,
        json!([{"send": {"passkeyPrompts": {}}}, {"send": {"passkeyConfirm": {"id": "1"}}}]),
    );
    register(&pretender, MENTION);
    pretender.go();
    assert_eq!(pretender.line(), sockets::not_permitted());
    assert_eq!(pretender.line(), sockets::not_permitted());
    pretender.end();
    drop(sheet);
    drop(client);

    // The session's own client, on a thread of its own: told the sheet, it answers for the person.
    // Unlocked again by the lock's check first.
    crate::session::check(
        &socket.path,
        Secret::new(PASSWORD),
        crate::platform::Local::Accepted,
    )
    .unwrap();
    sockets::wait_for("the keychain unlocked again", || {
        !matches!(
            keychains.settings(uid, &user, Request::KeychainStatus {}),
            Answer::KeychainStatus(Status {
                state: KeychainState::Locked,
                ..
            })
        )
    });
    let sheets = PasskeySheets::spawn(socket.path.clone(), |_| {}).unwrap();
    sockets::wait_for("the session's sheets served", || {
        keychains.shows_sheets(uid)
    });
    let mut threaded = Application::start(
        &socket,
        json!([{"send": get_request("mention.earth", "threaded", &[])}]),
    );
    register(&threaded, MENTION);
    threaded.go();
    let mut taken = Vec::new();
    sockets::wait_for("the sheet told", || {
        taken.extend(sheets.take());
        !taken.is_empty()
    });
    let Prompt::ConfirmPasskey(shown) = taken.remove(0) else {
        panic!("not a sheet");
    };
    assert_eq!(shown.kind, PromptKind::Get);
    sheets.confirm(&shown.id, Some(0), Some(Secret::new(PASSWORD)));
    assert!(verifies(
        &threaded.line(),
        &point,
        "mention.earth",
        "threaded"
    ));
    sockets::wait_for("the sheet done", || {
        taken.extend(sheets.take());
        taken.contains(&Prompt::PasskeyDone {
            id: shown.id.clone(),
        })
    });
    threaded.end();
    drop(sheets);
    socket.machine.platform.settle();
}

#[test]
fn a_passkey_made_on_one_machine_signs_on_another() {
    let (_oxy, base) = Oxy::start();
    let (dir_a, dir_b) = (
        TempDir::new("passkeys-sync-a"),
        TempDir::new("passkeys-sync-b"),
    );
    let a = machine(&base, "a");
    let b = machine(&base, "b");
    a.service.set_relying_parties(parties(&dir_a));
    b.service.set_relying_parties(parties(&dir_b));
    a.unlock(PASSWORD_A);
    b.unlock(PASSWORD_B);
    a.create();
    approve(&a, &b);
    let sheet = |machine: &Machine| {
        let (service_end, sheet_end) = Connection::pair().unwrap();
        let service = machine.service.clone();
        let (uid, user) = (machine.uid, machine.user.clone());
        std::thread::spawn(move || service.serve_prompts(uid, &user, &service_end));
        // Told it is open once its sheets are shown there.
        sheet_end
            .set_receive_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        assert_eq!(
            sheet_end.receive::<Answer>().unwrap().unwrap().unwrap(),
            Answer::PasskeyPromptsOpen
        );
        assert!(machine.service.shows_sheets(machine.uid));
        Pair(sheet_end)
    };
    let (sheet_a, sheet_b) = (sheet(&a), sheet(&b));
    let gone = || false;
    let asker = crate::keychain::Asker {
        desktop_entry_id: MENTION,
        pid: 1,
        gone: &gone,
    };
    // Made on a, the person verifying there.
    let request: Request =
        serde_json::from_value(create_request("mention.earth", &[42; 16], &[])).unwrap();
    let made = std::thread::scope(|scope| {
        let asked = scope.spawn(|| a.service.passkey(a.uid, &a.user, &asker, request));
        let shown = sheet_a.shown(PromptKind::Create, "mention.earth");
        sheet_a.confirm(&shown.id, None, Some(PASSWORD_A));
        sheet_a.done(&shown.id);
        asked.join().unwrap()
    });
    let created = serde_json::to_value(&made).unwrap();
    let point = registered(&created, "mention.earth");
    // One ceremony at a time for an application: another while the first waits is refused at
    // once, and the first goes on.
    let twice = || -> Request {
        serde_json::from_value(get_request("mention.earth", "twice", &[])).unwrap()
    };
    std::thread::scope(|scope| {
        let first = scope.spawn(|| a.service.passkey(a.uid, &a.user, &asker, twice()));
        let shown = sheet_a.shown(PromptKind::Get, "mention.earth");
        let second = a.service.passkey(a.uid, &a.user, &asker, twice());
        assert_eq!(serde_json::to_value(second).unwrap(), not_allowed());
        sheet_a.quiet();
        sheet_a.cancel(&shown.id);
        sheet_a.done(&shown.id);
        assert_eq!(
            serde_json::to_value(first.join().unwrap()).unwrap(),
            not_allowed()
        );
    });
    // Synced to b, and used there, b's person verifying with b's password.
    b.sync().unwrap();
    let request: Request =
        serde_json::from_value(get_request("mention.earth", "on b", &[])).unwrap();
    let asserted = std::thread::scope(|scope| {
        let asked = scope.spawn(|| b.service.passkey(b.uid, &b.user, &asker, request));
        let shown = sheet_b.shown(PromptKind::Get, "mention.earth");
        sheet_b.confirm(&shown.id, Some(0), Some(PASSWORD_A));
        assert!(matches!(sheet_b.prompt(), Prompt::PasskeyRetry { .. }));
        sheet_b.confirm(&shown.id, Some(0), Some(PASSWORD_B));
        sheet_b.done(&shown.id);
        asked.join().unwrap()
    });
    let asserted = serde_json::to_value(&asserted).unwrap();
    assert!(verifies(&asserted, &point, "mention.earth", "on b"));
    assert_eq!(
        asserted["passkeyAssertion"]["credentialId"],
        created["passkeyCreated"]["credentialId"]
    );
    // Settings on b reveals its relying party and user, never its key.
    b.verify(PASSWORD_B);
    let listed = b.list();
    assert_eq!(listed.len(), 1);
    let shown = serde_json::to_value(b.reveal(&listed[0].item_id)).unwrap();
    assert_eq!(shown["type"], "passkey");
    assert_eq!(shown["rpId"], "mention.earth");
    assert_eq!(shown["userName"], "ada@example.com");
    assert!(shown.get("privateKey").is_none());
}

#[test]
fn the_wire_shapes() {
    let confirm = Prompt::ConfirmPasskey(ConfirmPasskey {
        id: "1-abc".into(),
        desktop_entry_id: MENTION.into(),
        pid: 42,
        rp_id: "mention.earth".into(),
        user_name: Some(Secret::new("ada")),
        kind: PromptKind::Get,
        choices: vec![PasskeyChoice {
            user_name: Secret::new("ada"),
            display_name: Secret::new("Ada"),
        }],
    });
    assert_eq!(
        serde_json::to_value(&confirm).unwrap(),
        json!({"confirmPasskey": {"id": "1-abc", "desktopEntryId": MENTION, "pid": 42,
            "rpId": "mention.earth", "userName": "ada", "kind": "get",
            "choices": [{"userName": "ada", "displayName": "Ada"}]}})
    );
    assert_eq!(
        serde_json::to_value(Prompt::PasskeyRetry {
            id: "1".into(),
            reason: RetryReason::Throttled,
            retry_after: Some(30)
        })
        .unwrap(),
        json!({"passkeyRetry": {"id": "1", "reason": "throttled", "retryAfter": 30}})
    );
    assert_eq!(
        serde_json::to_value(Prompt::PasskeyDone { id: "1".into() }).unwrap(),
        json!({"passkeyDone": {"id": "1"}})
    );
    let confirm: Request = serde_json::from_value(
        json!({"passkeyConfirm": {"id": "1", "choice": 0, "password": "p"}}),
    )
    .unwrap();
    assert!(matches!(
        confirm,
        Request::PasskeyConfirm {
            choice: Some(0),
            ..
        }
    ));
    let create: Request = serde_json::from_value(create_request("a.example", &[1], &[])).unwrap();
    assert!(matches!(create, Request::PasskeyCreate(_)));
    // Unknown fields are refused, as the request's every field is the authenticator's.
    assert!(serde_json::from_value::<Request>(
        json!({"passkeyGet": {"rpId": "a.example", "clientDataHash": "x", "origin": "https://evil"}})
    )
    .is_err());
    for (reason, wire) in [
        (Reason::NotAllowed, "notAllowed"),
        (Reason::InvalidState, "invalidState"),
        (Reason::NotSupported, "notSupported"),
    ] {
        assert_eq!(
            serde_json::to_value(Answer::refused(reason)).unwrap(),
            json!({"refused": {"reason": wire}})
        );
    }
    // No password, name or user in Debug.
    let shown = format!(
        "{confirm:?} {:?}",
        Request::PasskeyConfirm {
            id: "1".into(),
            choice: None,
            password: Some(Secret::new("hunter2-secret"))
        }
    );
    assert!(
        !shown.contains("hunter2") && !shown.contains("\"p\""),
        "{shown}"
    );
}
