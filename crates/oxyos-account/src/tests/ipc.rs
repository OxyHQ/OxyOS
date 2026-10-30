//! The service's socket: a real `SOCK_SEQPACKET` socket in a directory of the test's own, the
//! service's side served on a thread, the setup's seams fakes.

use super::*;
use crate::ipc::{
    serve, Answer, Connection, Conversation, Enrollers, Listener, Outcome, Reason, Request, Turn,
};
use crate::setup::Setup;
use crate::store::Store;
use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;

fn me() -> u32 {
    rustix::process::getuid().as_raw()
}

fn enroll(password: &str) -> Request {
    Request::Enroll {
        device_id: DEVICE.into(),
        device_secret: Secret::new(DEVICE_SECRET),
        account_id: ACCOUNT.into(),
        password: Secret::new(password),
    }
}

/// A service on a socket of its own, serving `conversations` clients, each outcome logged.
struct Service {
    socket: std::path::PathBuf,
    setup: Arc<Setup<FakeApi, FakeUsers>>,
    log: Arc<Mutex<Vec<String>>>,
    thread: Option<std::thread::JoinHandle<()>>,
    _dir: TempDir,
}

impl Service {
    fn start(api: FakeApi, enrollers: Enrollers, conversations: usize) -> Self {
        let dir = TempDir::new("ipc");
        let store = Store::open(&dir.path().join("accounts")).unwrap();
        let setup = Arc::new(Setup::new(api, FakeUsers::default(), store));
        let socket = dir.path().join("socket");
        let listener = Listener::bind(&socket).unwrap();
        let log = Arc::new(Mutex::new(Vec::new()));
        let thread = {
            let setup = setup.clone();
            let log = log.clone();
            std::thread::spawn(move || {
                for _ in 0..conversations {
                    let connection = listener.accept().unwrap();
                    serve(&connection, &setup, enrollers, |peer, outcome| {
                        log.lock()
                            .unwrap()
                            .push(format!("{} {outcome:?}", peer.uid));
                    })
                    .unwrap();
                }
            })
        };
        Self {
            socket,
            setup,
            log,
            thread: Some(thread),
            _dir: dir,
        }
    }

    fn connect(&self) -> Connection {
        Connection::connect(&self.socket).unwrap()
    }

    /// Waits for the service's conversations to end; its directory stays until it is dropped.
    fn finish(&mut self) -> Vec<String> {
        self.thread.take().unwrap().join().unwrap();
        self.log.lock().unwrap().clone()
    }
}

fn answer(connection: &Connection) -> Answer {
    connection.receive::<Answer>().unwrap().unwrap().unwrap()
}

#[test]
fn only_root_and_the_login_programs_user_may_enroll() {
    let enrollers = Enrollers { login: Some(990) };
    assert!(enrollers.allow(0));
    assert!(enrollers.allow(990));
    assert!(!enrollers.allow(1000));
    let without_login = Enrollers { login: None };
    assert!(without_login.allow(0));
    assert!(!without_login.allow(990));
}

#[test]
fn a_client_enrolls_an_account_on_the_socket() {
    let mut service = Service::start(FakeApi::default(), Enrollers { login: Some(me()) }, 1);
    let mode = std::fs::metadata(&service.socket)
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(
        mode & 0o777,
        0o666,
        "the peer's credentials decide, not the mode"
    );
    let client = service.connect();
    client.send(&enroll(PASSWORD)).unwrap();
    let Answer::Enrolled {
        account_id,
        user,
        profile,
        ..
    } = answer(&client)
    else {
        panic!("enrolled");
    };
    assert_eq!(account_id, ACCOUNT);
    assert_eq!(user, account().user_name().as_str());
    assert_eq!(profile.username.as_deref(), Some(USERNAME));
    // The conversation is over.
    assert!(client.receive::<Answer>().unwrap().is_none());
    let setup = service.setup.clone();
    let log = service.finish();
    assert!(setup.store().get(&account()).unwrap().is_some());
    assert_eq!(log.len(), 1);
    assert!(log[0].contains("Enrolled"));
}

#[test]
fn a_peer_that_may_not_enroll_is_told_so_and_nothing_else() {
    // A login user that is not this process's (unless the tests run as root, which may).
    let mut service = Service::start(
        FakeApi::default(),
        Enrollers {
            login: Some(me().wrapping_add(1)),
        },
        1,
    );
    let client = service.connect();
    client.send(&enroll(PASSWORD)).unwrap();
    let reply = answer(&client);
    let setup = service.setup.clone();
    let log = service.finish();
    if me() == 0 {
        assert!(matches!(reply, Answer::Enrolled { .. }));
    } else {
        assert_eq!(reply, Answer::refused(Reason::NotPermitted));
        assert!(setup.api().calls().is_empty());
        assert!(log.is_empty());
    }
}

#[test]
fn a_second_factor_is_asked_for_on_the_same_connection() {
    let mut service = Service::start(
        FakeApi {
            authenticator: true,
            ..FakeApi::default()
        },
        Enrollers { login: Some(me()) },
        1,
    );
    let client = service.connect();
    client.send(&enroll(PASSWORD)).unwrap();
    assert_eq!(answer(&client), Answer::SecondFactorRequired);
    client
        .send(&Request::SecondFactor {
            code: Secret::new("111111"),
        })
        .unwrap();
    assert_eq!(
        answer(&client),
        Answer::refused(Reason::SecondFactorInvalid)
    );
    client
        .send(&Request::SecondFactor {
            code: Secret::new(CODE),
        })
        .unwrap();
    assert!(matches!(answer(&client), Answer::Enrolled { .. }));
    let log = service.finish();
    let log = log.join("\n");
    for secret in [PASSWORD, DEVICE_SECRET, CHALLENGE, CODE, "111111"] {
        assert!(!log.contains(secret), "{log}");
    }
}

#[test]
fn an_offline_setup_is_refused_on_the_socket_and_leaves_nothing() {
    let mut service = Service::start(
        FakeApi {
            offline_from: Some(Call::Mint),
            ..FakeApi::default()
        },
        Enrollers { login: Some(me()) },
        1,
    );
    let client = service.connect();
    client.send(&enroll(PASSWORD)).unwrap();
    assert_eq!(answer(&client), Answer::refused(Reason::Offline));
    let setup = service.setup.clone();
    service.finish();
    assert!(setup.users().names().is_empty());
    assert!(setup.store().users().unwrap().is_empty());
}

#[test]
fn a_client_that_goes_away_abandons_its_setup() {
    let mut service = Service::start(
        FakeApi {
            authenticator: true,
            ..FakeApi::default()
        },
        Enrollers { login: Some(me()) },
        1,
    );
    let client = service.connect();
    client.send(&enroll(PASSWORD)).unwrap();
    assert_eq!(answer(&client), Answer::SecondFactorRequired);
    drop(client);
    let setup = service.setup.clone();
    service.finish();
    assert!(setup.users().names().is_empty());
    assert!(setup.store().users().unwrap().is_empty());
}

#[test]
fn what_is_not_a_request_ends_the_conversation() {
    let mut service = Service::start(FakeApi::default(), Enrollers { login: Some(me()) }, 3);
    // Not JSON.
    let client = service.connect();
    client.send(&"not a request").unwrap();
    assert_eq!(answer(&client), Answer::refused(Reason::InvalidRequest));
    // Too long for a message.
    let client = service.connect();
    assert!(client
        .send(&enroll(&"x".repeat(crate::ipc::MAX_MESSAGE)))
        .is_err());
    let long = format!("{{\"enroll\":\"{}\"}}", "x".repeat(crate::ipc::MAX_MESSAGE));
    rustix::net::send(&client, long.as_bytes(), rustix::net::SendFlags::empty()).unwrap();
    assert_eq!(answer(&client), Answer::refused(Reason::InvalidRequest));
    // A code before any setup.
    let client = service.connect();
    client
        .send(&Request::SecondFactor {
            code: Secret::new(CODE),
        })
        .unwrap();
    assert_eq!(answer(&client), Answer::refused(Reason::InvalidRequest));
    let setup = service.setup.clone();
    service.finish();
    assert!(setup.api().calls().is_empty());
}

#[test]
fn a_conversation_takes_one_setup() {
    let dir = TempDir::new("ipc-conversation");
    let setup = Setup::new(
        FakeApi::default(),
        FakeUsers::default(),
        Store::open(&dir.path().join("accounts")).unwrap(),
    );
    let mut conversation = Conversation::new(&setup);
    let (turn, outcome) = conversation.answer(Some(enroll(PASSWORD)));
    assert!(matches!(turn, Turn::End(Answer::Enrolled { .. })));
    assert!(matches!(outcome, Outcome::Enrolled(_)));
    let (turn, _) = conversation.answer(Some(enroll(PASSWORD)));
    assert_eq!(turn, Turn::End(Answer::refused(Reason::InvalidRequest)));
}

#[test]
fn the_socket_replaces_an_old_socket_never_another_file() {
    let dir = TempDir::new("ipc-bind");
    let socket = dir.path().join("socket");
    drop(Listener::bind(&socket).unwrap());
    Listener::bind(&socket).unwrap();
    let file = dir.path().join("file");
    std::fs::write(&file, "keep").unwrap();
    assert!(Listener::bind(&file).is_err());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "keep");
}

#[test]
fn the_wire_format_is_the_documented_one() {
    let request: Request = serde_json::from_str(
        r#"{"enroll":{"deviceId":"d","deviceSecret":"s","accountId":"a","password":"p"}}"#,
    )
    .unwrap();
    assert!(matches!(request, Request::Enroll { .. }));
    assert_eq!(
        serde_json::to_string(&Answer::SecondFactorRequired).unwrap(),
        r#""secondFactorRequired""#
    );
    assert_eq!(
        serde_json::to_string(&Answer::Refused {
            reason: Reason::Locked,
            retry_after_seconds: Some(900)
        })
        .unwrap(),
        r#"{"refused":{"reason":"locked","retryAfterSeconds":900}}"#
    );
    assert_eq!(
        serde_json::to_string(&Answer::refused(Reason::NotPermitted)).unwrap(),
        r#"{"refused":{"reason":"notPermitted"}}"#
    );
    let printed = format!("{:?}", enroll(PASSWORD));
    assert!(!printed.contains(PASSWORD) && !printed.contains(DEVICE_SECRET));
}
