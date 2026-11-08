//! The service's socket: a real `SOCK_SEQPACKET` socket in a directory of the test's own, the
//! service's side served on a thread, the setup's seams fakes.

use super::*;
use crate::ipc::Failure;
use crate::ipc::{
    list, serve, Answer, Connection, Conversation, Enrollers, Listener, Outcome, Reason, Request,
    Turn,
};
use crate::platform::Platform;
use crate::setup::{Progress, Setup, Stage};
use crate::store::Store;
use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;

/// Oxy's API, as the service tells the login screen.
const API: &str = "http://127.0.0.1:9";

fn me() -> u32 {
    rustix::process::getuid().as_raw()
}

fn enroll(password: &str) -> Request {
    Request::Enroll {
        device_id: DEVICE.into(),
        device_secret: Secret::new(DEVICE_SECRET),
        account_id: ACCOUNT.into(),
        password: Some(Secret::new(password)),
        pin: None,
        appearance: None,
    }
}

/// A service on a socket of its own, serving `conversations` clients, each outcome logged.
struct Service {
    socket: std::path::PathBuf,
    platform: Arc<Platform<FakeApi, FakeUsers>>,
    log: Arc<Mutex<Vec<String>>>,
    thread: Option<std::thread::JoinHandle<()>>,
    _dir: TempDir,
}

impl Service {
    fn start(api: FakeApi, enrollers: Enrollers, conversations: usize) -> Self {
        let dir = TempDir::new("ipc");
        let store = Store::open(&dir.path().join("accounts")).unwrap();
        let platform = Arc::new(Platform::new(
            Setup::new(api, FakeUsers::default(), store).with_platform(platform_client()),
        ));
        let socket = dir.path().join("socket");
        let listener = Listener::bind(&socket).unwrap();
        let log = Arc::new(Mutex::new(Vec::new()));
        let thread = {
            let platform = platform.clone();
            let log = log.clone();
            std::thread::spawn(move || {
                for _ in 0..conversations {
                    let connection = listener.accept().unwrap();
                    serve(&connection, &platform, API, enrollers, |peer, outcome| {
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
            platform,
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

/// The next answer that is not a step's progress.
fn answer(connection: &Connection) -> Answer {
    stages_and_answer(connection).1
}

/// The steps told, then the answer.
fn stages_and_answer(connection: &Connection) -> (Vec<Stage>, Answer) {
    let mut stages = Vec::new();
    loop {
        match connection.receive::<Answer>().unwrap().unwrap().unwrap() {
            Answer::Progress(stage) => stages.push(stage),
            answer => return (stages, answer),
        }
    }
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
    let platform = service.platform.clone();
    let setup = platform.setup();
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
    let platform = service.platform.clone();
    let setup = platform.setup();
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
    assert_eq!(
        answer(&client),
        Answer::Failed {
            reason: Failure::Unreachable
        },
        "the sign-in is kept: the setup may try again"
    );
    drop(client);
    let platform = service.platform.clone();
    let setup = platform.setup();
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
    let platform = service.platform.clone();
    let setup = platform.setup();
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
    let platform = service.platform.clone();
    let setup = platform.setup();
    service.finish();
    assert!(setup.api().calls().is_empty());
}

#[test]
fn a_conversation_takes_one_setup() {
    let dir = TempDir::new("ipc-conversation");
    let platform = Arc::new(Platform::new(
        Setup::new(
            FakeApi::default(),
            FakeUsers::default(),
            Store::open(&dir.path().join("accounts")).unwrap(),
        )
        .with_platform(platform_client()),
    ));
    let mut conversation = Conversation::new(&platform, API);
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
fn the_socket_appears_only_listening() {
    use rustix::fs::inotify::{self, CreateFlags, ReadFlags, WatchFlags};
    let dir = TempDir::new("ipc-listening");
    let socket = dir.path().join("socket");
    let watch = inotify::init(CreateFlags::NONBLOCK | CreateFlags::CLOEXEC).unwrap();
    inotify::add_watch(
        &watch,
        dir.path(),
        WatchFlags::CREATE | WatchFlags::MOVED_TO,
    )
    .unwrap();
    // Made, then made again over the one a run before left: each time the name appears only by
    // a rename of a socket already listening, never by its bind (a connection between a bind and
    // its listen would be refused).
    for _ in 0..2 {
        let listener = Listener::bind(&socket).unwrap();
        let mut buffer = [std::mem::MaybeUninit::<u8>::uninit(); 4096];
        let mut reader = inotify::Reader::new(&watch, &mut buffer);
        let mut named = Vec::new();
        while let Ok(event) = reader.next() {
            if event.file_name().map(|name| name.to_bytes()) == Some(b"socket".as_slice()) {
                named.push(event.events());
            }
        }
        assert_eq!(named, [ReadFlags::MOVED_TO]);
        let connected = Connection::connect(&socket).unwrap();
        let accepted = listener.accept().unwrap();
        drop((connected, accepted));
    }
    // Nothing of the making is left beside it.
    let left: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .flatten()
        .map(|entry| entry.file_name())
        .collect();
    assert_eq!(left, [std::ffi::OsString::from("socket")]);
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

#[test]
fn the_login_screen_lists_the_enrolled_accounts_and_others_may_not() {
    let mut service = Service::start(FakeApi::default(), Enrollers { login: Some(me()) }, 3);
    assert_eq!(list(&service.socket).unwrap(), Vec::new());
    let client = service.connect();
    client.send(&enroll(PASSWORD)).unwrap();
    assert!(matches!(answer(&client), Answer::Enrolled { .. }));
    let accounts = list(&service.socket).unwrap();
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0].account_id, ACCOUNT);
    assert_eq!(accounts[0].user, account().user_name().as_str());
    assert_eq!(accounts[0].picture, None);
    let log = service.finish();
    assert!(log[2].contains("Listed { accounts: 1"), "{log:?}");
    // Not the login program's user (unless the tests run as root, which may).
    let mut service = Service::start(
        FakeApi::default(),
        Enrollers {
            login: Some(me().wrapping_add(1)),
        },
        1,
    );
    let refused = list(&service.socket);
    service.finish();
    if me() != 0 {
        assert_eq!(
            refused.unwrap_err().kind(),
            std::io::ErrorKind::PermissionDenied
        );
    }
}

#[test]
fn the_list_is_the_documented_one() {
    assert_eq!(serde_json::to_string(&Request::List).unwrap(), r#""list""#);
    let answer = Answer::Accounts(vec![crate::ipc::Account {
        account_id: "a".into(),
        user: "oxy-a".into(),
        uid: 1001,
        name: "Ada".into(),
        picture: None,
        state: crate::store::State::SignInAgain,
        pin: crate::pin::PinState::Locked,
        password: false,
    }]);
    assert_eq!(
        serde_json::to_string(&answer).unwrap(),
        r#"{"accounts":[{"accountId":"a","user":"oxy-a","uid":1001,"name":"Ada","state":"signInAgain","pin":"locked","password":false}]}"#
    );
    // What an older service lists, without a state, is ready.
    let older: crate::ipc::Account =
        serde_json::from_str(r#"{"accountId":"a","user":"oxy-a","uid":1001,"name":"Ada"}"#)
            .unwrap();
    assert_eq!(older.state, crate::store::State::Ready);
}

#[test]
fn the_login_screen_is_told_where_oxy_is() {
    let mut service = Service::start(FakeApi::default(), Enrollers { login: Some(me()) }, 1);
    let platform = crate::ipc::platform(&service.socket).unwrap();
    service.finish();
    assert_eq!(
        platform,
        crate::ipc::PlatformConfig {
            api: API.into(),
            client_id: Some(CLIENT_ID.into()),
        }
    );
    assert_eq!(
        serde_json::to_string(&Answer::Platform(platform)).unwrap(),
        format!(r#"{{"platform":{{"api":"{API}","clientId":"{CLIENT_ID}"}}}}"#)
    );
    assert_eq!(
        serde_json::to_string(&Request::Platform).unwrap(),
        r#""platform""#
    );
}

#[test]
fn the_login_program_checks_a_password_on_the_socket() {
    use crate::platform::{CheckReason, Checked, Local, Verdict};
    let mut service = Service::start(FakeApi::default(), Enrollers { login: Some(me()) }, 4);
    let client = service.connect();
    client.send(&enroll(PASSWORD)).unwrap();
    assert!(matches!(answer(&client), Answer::Enrolled { .. }));
    let user = account().user_name();
    // PAM accepted it: accepted at once.
    assert_eq!(
        crate::ipc::check(
            &service.socket,
            user.as_str(),
            Secret::new(PASSWORD),
            Local::Accepted
        )
        .unwrap(),
        Checked::ACCEPTED
    );
    // PAM rejected a wrong one, and so does Oxy.
    assert_eq!(
        crate::ipc::check(
            &service.socket,
            user.as_str(),
            Secret::new("wrong"),
            Local::Rejected
        )
        .unwrap(),
        Checked::REJECTED
    );
    // An account that is not enrolled is rejected.
    assert_eq!(
        crate::ipc::check(
            &service.socket,
            "oxy-aaaa",
            Secret::new(PASSWORD),
            Local::Accepted
        )
        .unwrap(),
        Checked::REJECTED
    );
    let platform = service.platform.clone();
    let log = service.finish();
    platform.settle();
    let log = log.join("\n");
    assert!(!log.contains(PASSWORD), "{log}");
    // The wire shapes.
    let request: Request =
        serde_json::from_str(r#"{"check":{"user":"oxy-a","password":"p","local":"rejected"}}"#)
            .unwrap();
    assert!(matches!(
        request,
        Request::Check {
            local: Local::Rejected,
            ..
        }
    ));
    assert_eq!(
        serde_json::to_string(&Answer::Checked(Checked {
            result: Verdict::Rejected,
            reason: Some(CheckReason::SignInAgain),
        }))
        .unwrap(),
        r#"{"checked":{"result":"rejected","reason":"signInAgain"}}"#
    );
    assert_eq!(
        serde_json::to_string(&Answer::Checked(Checked::ACCEPTED)).unwrap(),
        r#"{"checked":{"result":"accepted"}}"#
    );
}

#[cfg(feature = "local")]
#[test]
fn a_pin_is_checked_for_its_own_account_alone() {
    use crate::ipc::{verify_pin_at, PinResult};
    // Not the login program's user: an ordinary peer, as the lock's PAM runs.
    let login = Some(me().wrapping_add(1));
    let mut service = Service::start(FakeApi::default(), Enrollers { login }, 4);
    let setup = service.platform.setup();
    let mut enrolled = None;
    if let Progress::Enrolled(done) = setup
        .enroll(crate::setup::Request {
            credential: credential(),
            account: ACCOUNT.into(),
            password: Some(Secret::new(PASSWORD)),
            pin: Some(Secret::new("482916")),
            appearance: None,
        })
        .unwrap()
    {
        enrolled = Some(done);
    }
    let user = enrolled.unwrap().user;
    let mut stored = setup.store().get(&account()).unwrap().unwrap();
    stored.uid = me().wrapping_add(7);
    setup.store().put(&stored).unwrap();
    // Another uid's account: answered as one without a PIN, nothing checked or counted.
    let other = verify_pin_at(&service.socket, user.as_str(), Secret::new("000000")).unwrap();
    // Root may check any account's (its first wrong one counted).
    let counted = if me() == 0 {
        assert_eq!(other.result, PinResult::Rejected);
        1
    } else {
        assert_eq!(other.result, PinResult::None);
        0
    };
    assert_eq!(
        setup.store().local(&user).unwrap().pin.unwrap().failures,
        counted
    );
    // The peer's own account.
    let mut stored = setup.store().get(&account()).unwrap().unwrap();
    stored.uid = me();
    setup.store().put(&stored).unwrap();
    let wrong = verify_pin_at(&service.socket, user.as_str(), Secret::new("000000")).unwrap();
    assert_eq!(wrong.result, PinResult::Rejected);
    assert_eq!(
        setup.store().local(&user).unwrap().pin.unwrap().failures,
        counted + 1
    );
    let right = verify_pin_at(&service.socket, user.as_str(), Secret::new("482916")).unwrap();
    assert_eq!(right.result, PinResult::Accepted);
    assert_eq!(setup.store().local(&user).unwrap().pin.unwrap().failures, 0);
    // Anything else of such a peer is refused.
    if me() != 0 {
        assert_eq!(
            list(&service.socket).unwrap_err().kind(),
            std::io::ErrorKind::PermissionDenied
        );
    } else {
        list(&service.socket).unwrap();
    }
    let log = service.finish();
    assert!(
        log.iter().any(|line| line.contains("PinChecked")),
        "{log:?}"
    );
    assert!(log.iter().all(|line| !line.contains("482916")));
}

#[test]
fn the_pin_wire_format_is_the_documented_one() {
    let request = Request::VerifyPin {
        user: "oxy-a".into(),
        pin: Secret::new("1234"),
    };
    assert_eq!(
        serde_json::to_string(&request).unwrap(),
        r#"{"verifyPin":{"user":"oxy-a","pin":"1234"}}"#
    );
    let answer = Answer::Pin(crate::ipc::PinAnswer {
        result: crate::ipc::PinResult::Throttled,
        retry_after_seconds: Some(30),
    });
    assert_eq!(
        serde_json::to_string(&answer).unwrap(),
        r#"{"pin":{"result":"throttled","retryAfterSeconds":30}}"#
    );
    // An enroll without a password says so by leaving it out.
    let enroll = Request::Enroll {
        device_id: "d".into(),
        device_secret: Secret::new("s"),
        account_id: "a".into(),
        password: None,
        pin: Some(Secret::new("1234")),
        appearance: None,
    };
    assert_eq!(
        serde_json::to_string(&enroll).unwrap(),
        r#"{"enroll":{"deviceId":"d","deviceSecret":"s","accountId":"a","pin":"1234"}}"#
    );
}

/// The service's last message, sent just before it closes, is read before the end, with a
/// descriptor or without: the kernel can report the end with it still queued
/// (`oxyos-seqpacket`). Many rounds, since each one is a race. The client polls (its socket
/// non-blocking), where the race is widest; a waiting receive goes through the same read. Each
/// receive here zeroes a message's buffer, so few rounds meet the race: `oxyos-seqpacket`'s own
/// test is the one that meets it often.
#[test]
fn the_last_message_before_the_end_is_read_before_the_end() {
    fn polled<T>(receive: impl Fn() -> std::io::Result<T>) -> T {
        loop {
            match receive() {
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::hint::spin_loop()
                }
                received => break received.unwrap(),
            }
        }
    }
    let dir = TempDir::new("last-message");
    let socket = dir.path().join("socket");
    let listener = Listener::bind(&socket).unwrap();
    for round in 0..2000u32 {
        let client = Connection::connect(&socket).unwrap();
        rustix::io::ioctl_fionbio(std::os::fd::AsFd::as_fd(&client), true).unwrap();
        let service = listener.accept().unwrap();
        let with_descriptor = round % 2 == 1;
        let sender = std::thread::spawn(move || {
            if with_descriptor {
                let null = std::fs::File::open("/dev/null").unwrap();
                service
                    .send_with(&round, std::os::fd::AsFd::as_fd(&null))
                    .unwrap();
            } else {
                service.send(&round).unwrap();
            }
            // Shut down, not only closed: a process another test forks meanwhile may hold a copy
            // of the descriptor until it executes.
            rustix::net::shutdown(
                std::os::fd::AsFd::as_fd(&service),
                rustix::net::Shutdown::Both,
            )
            .unwrap();
        });
        let heard = if with_descriptor {
            polled(|| client.receive_with::<u32>()).map(|(message, fds)| (message, fds.len()))
        } else {
            polled(|| client.receive::<u32>()).map(|message| (message, 0))
        };
        assert_eq!(
            heard,
            Some((Some(round), usize::from(with_descriptor))),
            "round {round}"
        );
        sender.join().unwrap();
        assert!(polled(|| client.receive::<u32>()).is_none());
    }
}

/// A setup by the QR, as the login screen sends it: its PIN, no password.
#[cfg(feature = "local")]
fn enroll_by_qr(appearance: Option<crate::setup::Appearance>) -> Request {
    Request::Enroll {
        device_id: DEVICE.into(),
        device_secret: Secret::new(DEVICE_SECRET),
        account_id: ACCOUNT.into(),
        password: None,
        pin: Some(Secret::new("482916")),
        appearance,
    }
}

#[cfg(feature = "local")]
#[test]
fn a_setup_tells_its_steps_before_its_answer() {
    let mut service = Service::start(FakeApi::default(), Enrollers { login: Some(me()) }, 1);
    let client = service.connect();
    client
        .send(&enroll_by_qr(Some(crate::setup::Appearance::Dark)))
        .unwrap();
    let (stages, answer) = stages_and_answer(&client);
    assert_eq!(
        stages,
        [
            Stage::Securing,
            Stage::Verifying,
            Stage::Preparing,
            Stage::SigningIn,
            Stage::Finishing,
            Stage::Confirming
        ]
    );
    assert!(matches!(answer, Answer::Enrolled { .. }), "{answer:?}");
    let platform = service.platform.clone();
    let setup = platform.setup();
    service.finish();
    assert_eq!(setup.users().seeded.lock().unwrap().len(), 1);
}

#[cfg(feature = "local")]
#[test]
fn a_failed_setup_keeps_its_sign_in_and_a_retry_finishes_it() {
    let mut service = Service::start(FakeApi::default(), Enrollers { login: Some(me()) }, 1);
    let platform = service.platform.clone();
    let setup = platform.setup();
    setup.users().fail(UserStep::MakeAdministrator);
    let client = service.connect();
    client.send(&enroll_by_qr(None)).unwrap();
    assert_eq!(
        answer(&client),
        Answer::Failed {
            reason: Failure::LocalFailure
        }
    );
    setup.users().failing.lock().unwrap().clear();
    client.send(&Request::Retry).unwrap();
    let (stages, answer) = stages_and_answer(&client);
    assert_eq!(
        stages,
        [Stage::Preparing, Stage::Finishing, Stage::Confirming]
    );
    assert!(matches!(answer, Answer::Enrolled { .. }), "{answer:?}");
    let log = service.finish();
    // One redemption, and nothing ended online.
    let calls = setup.api().calls();
    assert_eq!(
        calls
            .iter()
            .filter(|call| **call == Call::PlatformDevice)
            .count(),
        1
    );
    assert!(!calls.contains(&Call::SignOut));
    assert!(log[0].contains("Interrupted"));
    assert!(log[1].contains("Enrolled"));
    let log = log.join("\n");
    for secret in [DEVICE_SECRET, "482916", &platform_secret(1)] {
        assert!(!log.contains(secret), "{log}");
    }
}

#[cfg(feature = "local")]
#[test]
fn a_client_that_goes_away_after_a_failure_ends_nothing_online() {
    let mut service = Service::start(FakeApi::default(), Enrollers { login: Some(me()) }, 1);
    let platform = service.platform.clone();
    let setup = platform.setup();
    setup.users().fail(UserStep::MakeAdministrator);
    let client = service.connect();
    client.send(&enroll_by_qr(None)).unwrap();
    assert!(matches!(answer(&client), Answer::Failed { .. }));
    drop(client);
    service.finish();
    // The credential it was issued is pending at Oxy (R8): it expires by itself.
    let calls = setup.api().calls();
    assert!(!calls.contains(&Call::SignOut));
    assert!(!calls.contains(&Call::Confirm));
    assert!(setup.api().is_pending(&platform_secret(1)));
    assert!(setup.users().names().is_empty());
    assert!(setup.store().users().unwrap().is_empty());
}

#[test]
fn a_retry_out_of_turn_and_an_unknown_appearance_are_not_requests() {
    let mut service = Service::start(FakeApi::default(), Enrollers { login: Some(me()) }, 2);
    let client = service.connect();
    client.send(&Request::Retry).unwrap();
    assert_eq!(answer(&client), Answer::refused(Reason::InvalidRequest));
    let client = service.connect();
    let mut message = serde_json::to_value(enroll(PASSWORD)).unwrap();
    message["enroll"]["appearance"] = serde_json::json!("sepia");
    client
        .send(&serde_json::from_value::<serde_json::Value>(message).unwrap())
        .unwrap();
    assert_eq!(answer(&client), Answer::refused(Reason::InvalidRequest));
    let platform = service.platform.clone();
    let setup = platform.setup();
    service.finish();
    assert!(setup.api().calls().is_empty());
}

#[cfg(feature = "local")]
#[test]
fn a_wait_oxy_asks_for_is_told_before_the_setup_goes_on() {
    let api = FakeApi::default();
    api.limited.lock().unwrap().push_back(Some(1));
    let mut service = Service::start(api, Enrollers { login: Some(me()) }, 1);
    let client = service.connect();
    client.send(&enroll_by_qr(None)).unwrap();
    let mut told = Vec::new();
    let answer = loop {
        match client.receive::<Answer>().unwrap().unwrap().unwrap() {
            event @ (Answer::Progress(_) | Answer::Waiting { .. }) => told.push(event),
            answer => break answer,
        }
    };
    assert!(matches!(answer, Answer::Enrolled { .. }), "{answer:?}");
    let waiting = told
        .iter()
        .position(|event| {
            *event
                == Answer::Waiting {
                    retry_after_seconds: 1,
                }
        })
        .expect("the wait was told");
    assert_eq!(told[waiting - 1], Answer::Progress(Stage::SigningIn));
    assert_eq!(told.last(), Some(&Answer::Progress(Stage::Confirming)));
    // On the wire as the login screen reads it.
    assert_eq!(
        serde_json::to_value(&told[waiting]).unwrap(),
        serde_json::json!({"waiting": {"retryAfterSeconds": 1}})
    );
    service.finish();
}
