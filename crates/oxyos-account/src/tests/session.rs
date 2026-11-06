//! The session socket with real processes: the test is the registrar (its program stands for the
//! session program, its user for an enrolled account's), and applications are child processes of
//! its own (a short Python program speaking the socket), registered by pidfd as the session
//! registers what it launches. Oxy and the users are fakes.

use super::platform::{machine_as, Machine};
use super::*;
use crate::pidfd;
use crate::platform::{Checked, Local, Status};
use crate::session::{Answer, Reason, Registrar, RegistrarThread, Registrars, Request, Sessions};
use rustix::fd::AsFd;
use std::io::{BufRead, BufReader, Write as _};
use std::path::PathBuf;
use std::process::{Child, ChildStdout};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn me() -> u32 {
    rustix::process::getuid().as_raw()
}

struct Socket {
    path: PathBuf,
    sessions: Arc<Sessions<FakeApi, FakeUsers>>,
    machine: Machine,
}

/// A service whose session socket serves every connection on a thread of its own, Ada enrolled
/// as this test's user.
fn service(api: FakeApi) -> Socket {
    let machine = machine_as(api, PASSWORD, Some(me()));
    let path = machine.dir.path().join("session");
    let listener = crate::ipc::Listener::bind(&path).unwrap();
    let sessions = Arc::new(Sessions::new(
        machine.platform.clone(),
        Registrars {
            program: std::env::current_exe().unwrap(),
            owner: me(),
        },
    ));
    let serving = sessions.clone();
    std::thread::spawn(move || loop {
        let Ok(connection) = listener.accept() else {
            return;
        };
        let kind = serving.classify(&connection);
        let serving = serving.clone();
        std::thread::spawn(move || serving.serve(&connection, kind, |_| {}));
    });
    Socket {
        path,
        sessions,
        machine,
    }
}

/// An application: a child of the test that, once told (a line on its standard input), connects
/// and runs `steps`: `{"send": message}` sends it and prints the answer, `"recv"` prints the next
/// message, `"wait"` waits for another line; `{"child": steps}` runs them in a child of its own.
const APPLICATION: &str = r#"
import json, socket, subprocess, sys
def run(path, steps):
    s = None
    for step in steps:
        if step == "wait":
            sys.stdin.readline()
            continue
        if isinstance(step, dict) and "child" in step:
            child = subprocess.Popen([sys.executable, "-I", "-c", sys.argv[3], path, json.dumps(step["child"])],
                                     stdin=subprocess.DEVNULL, stdout=sys.stdout)
            child.wait()
            continue
        if s is None:
            s = socket.socket(socket.AF_UNIX, socket.SOCK_SEQPACKET)
            s.connect(path)
        if step == "recv":
            data = s.recv(65536)
        else:
            s.send(json.dumps(step["send"]).encode())
            data = s.recv(65536)
        print(data.decode() if data else "closed", flush=True)
if len(sys.argv) > 2:
    if sys.stdin is not None and not sys.stdin.closed:
        try:
            sys.stdin.readline()
        except Exception:
            pass
    run(sys.argv[1], json.loads(sys.argv[2]))
"#;

struct Application {
    child: Child,
    output: BufReader<ChildStdout>,
}

impl Application {
    fn start(socket: &Socket, steps: serde_json::Value) -> Self {
        let mut child = std::process::Command::new("python3")
            .args(["-I", "-c", APPLICATION])
            .arg(&socket.path)
            .arg(steps.to_string())
            .arg(APPLICATION)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let output = BufReader::new(child.stdout.take().unwrap());
        Self { child, output }
    }

    fn pidfd(&self) -> rustix::fd::OwnedFd {
        pidfd::open(self.child.id()).unwrap()
    }

    /// Lets it go on (connect, or past a "wait").
    fn go(&mut self) {
        let stdin = self.child.stdin.as_mut().unwrap();
        stdin.write_all(b"go\n").unwrap();
        stdin.flush().unwrap();
    }

    fn line(&mut self) -> serde_json::Value {
        let mut line = String::new();
        self.output.read_line(&mut line).unwrap();
        if line.trim() == "closed" {
            return serde_json::Value::Null;
        }
        serde_json::from_str(line.trim()).unwrap_or_else(|_| panic!("not JSON: {line:?}"))
    }

    fn end(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn not_permitted() -> serde_json::Value {
    serde_json::json!({"refused": {"reason": "notPermitted"}})
}

fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(Instant::now() < deadline, "waited too long for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn a_registered_application_gets_the_snapshot_and_its_own_tokens() {
    let socket = service(FakeApi::default());
    let registrar = Registrar::connect(&socket.path).unwrap();
    let mut app = Application::start(
        &socket,
        serde_json::json!([
            {"send": "snapshot"},
            {"send": {"token": {"reason": "boot"}}},
            {"send": {"token": {"reason": "preflight"}}},
            {"send": {"token": {"reason": "response-401"}}},
        ]),
    );
    assert_eq!(
        registrar
            .register("so.oxy.dropsea", false, app.pidfd().as_fd())
            .unwrap(),
        Ok(())
    );
    app.go();
    let snapshot = app.line();
    assert_eq!(snapshot["snapshot"]["status"], "signed_in");
    assert_eq!(snapshot["snapshot"]["account"]["id"], ACCOUNT);
    assert_eq!(snapshot["snapshot"]["account"]["handle"], USERNAME);
    let first = app.line();
    assert_eq!(first["token"]["accessToken"], "app-token-1");
    assert!(first["token"]["expiresAt"].as_str().unwrap().ends_with('Z'));
    // Cached, then minted again when it was refused.
    assert_eq!(app.line()["token"]["accessToken"], "app-token-1");
    assert_eq!(app.line()["token"]["accessToken"], "app-token-2");
    assert_eq!(
        *socket.machine.api().minted.lock().unwrap(),
        [
            Attestation::Entry("so.oxy.dropsea".into()),
            Attestation::Entry("so.oxy.dropsea".into())
        ]
    );
    app.end();
}

#[test]
fn settings_and_the_shell_registered_as_system_applications_get_oxyoss_own_token() {
    let socket = service(FakeApi::default());
    let registrar = Registrar::connect(&socket.path).unwrap();
    for (id, system) in [
        ("so.oxy.settings", true),
        ("so.oxy.shell", true),
        ("so.oxy.settings", false),
    ] {
        let mut app = Application::start(
            &socket,
            serde_json::json!([{"send": {"token": {"reason": "boot"}}}]),
        );
        registrar
            .register(id, system, app.pidfd().as_fd())
            .unwrap()
            .unwrap();
        app.go();
        assert!(app.line()["token"]["accessToken"].is_string());
        app.end();
    }
    // Settings and the shell share OxyOS's own token (one application, one cache).
    assert_eq!(
        *socket.machine.api().minted.lock().unwrap(),
        [
            Attestation::OwnPlatform,
            Attestation::Entry("so.oxy.settings".into())
        ]
    );
}

#[test]
fn peers_that_are_not_registered_applications_are_refused() {
    let socket = service(FakeApi::default());
    let registrar = Registrar::connect(&socket.path).unwrap();
    // A process nobody registered.
    let mut stranger = Application::start(&socket, serde_json::json!([{"send": "snapshot"}]));
    stranger.go();
    assert_eq!(stranger.line(), not_permitted());
    stranger.end();
    // A child of a registered one.
    let mut parent = Application::start(
        &socket,
        serde_json::json!([{"child": [{"send": {"token": {"reason": "boot"}}}]}]),
    );
    registrar
        .register("so.oxy.dropsea", false, parent.pidfd().as_fd())
        .unwrap()
        .unwrap();
    parent.go();
    assert_eq!(parent.line(), not_permitted());
    parent.end();
    // The registrar is no application: it may not ask for a token.
    let answer = crate::session::ask(
        &socket.path,
        &Request::Token {
            reason: crate::platform::TokenReason::Boot,
        },
        Duration::from_secs(5),
    )
    .unwrap();
    assert_eq!(answer, Answer::refused(Reason::NotPermitted));
    assert!(socket.machine.api().minted.lock().unwrap().is_empty());
}

#[test]
fn only_the_registrars_own_children_are_registered() {
    let socket = service(FakeApi::default());
    let registrar = Registrar::connect(&socket.path).unwrap();
    // A process that is not its child: the test's parent.
    let parent = rustix::process::getppid();
    if let Some(Ok(pidfd)) = parent.map(|parent| pidfd::open(parent.as_raw_nonzero().get() as u32))
    {
        assert_eq!(
            registrar
                .register("so.oxy.dropsea", false, pidfd.as_fd())
                .unwrap(),
            Err(Reason::NotPermitted)
        );
    }
    // A descriptor that is no pidfd, and an id that is no desktop entry id.
    let file = std::fs::File::open("/dev/null").unwrap();
    assert_eq!(
        registrar
            .register("so.oxy.dropsea", false, file.as_fd())
            .unwrap(),
        Err(Reason::InvalidRequest)
    );
    let app = Application::start(&socket, serde_json::json!([]));
    assert_eq!(
        registrar
            .register("../etc", false, app.pidfd().as_fd())
            .unwrap(),
        Err(Reason::InvalidRequest)
    );
    assert!(socket.sessions.registered().is_empty());
    app.end();
}

#[test]
fn a_registration_lives_while_its_process_and_its_registrars_connection_do() {
    let socket = service(FakeApi::default());
    let registrar = Registrar::connect(&socket.path).unwrap();
    // Its process exits: forgotten.
    let app = Application::start(&socket, serde_json::json!([]));
    registrar
        .register("so.oxy.dropsea", false, app.pidfd().as_fd())
        .unwrap()
        .unwrap();
    assert_eq!(socket.sessions.registered().len(), 1);
    app.end();
    wait_for("the exited process forgotten", || {
        socket.sessions.registered().is_empty()
    });
    // Its registrar's connection closes: every registration made on it is dropped.
    let mut app = Application::start(&socket, serde_json::json!([{"send": "snapshot"}]));
    registrar
        .register("so.oxy.dropsea", false, app.pidfd().as_fd())
        .unwrap()
        .unwrap();
    drop(registrar);
    wait_for("the registrar's registrations dropped", || {
        socket.sessions.registered().is_empty()
    });
    app.go();
    assert_eq!(app.line(), not_permitted());
    app.end();
}

#[test]
fn offline_the_token_is_null_and_the_snapshot_signed_in() {
    let socket = service(FakeApi {
        offline_from: Some(Call::AppToken),
        ..FakeApi::default()
    });
    let registrar = Registrar::connect(&socket.path).unwrap();
    let mut app = Application::start(
        &socket,
        serde_json::json!([
            {"send": {"token": {"reason": "boot"}}},
            {"send": "snapshot"},
        ]),
    );
    registrar
        .register("so.oxy.dropsea", false, app.pidfd().as_fd())
        .unwrap()
        .unwrap();
    app.go();
    assert_eq!(app.line(), serde_json::json!({"token": null}));
    let snapshot = app.line();
    assert_eq!(snapshot["snapshot"]["status"], "signed_in");
    assert_eq!(snapshot["snapshot"]["online"], false);
    app.end();
}

#[test]
fn a_subscriber_is_told_every_change() {
    let socket = service(FakeApi::default());
    let registrar = Registrar::connect(&socket.path).unwrap();
    let mut app = Application::start(
        &socket,
        serde_json::json!([{"send": "subscribe"}, "recv", "recv"]),
    );
    registrar
        .register("so.oxy.dropsea", false, app.pidfd().as_fd())
        .unwrap()
        .unwrap();
    app.go();
    let first = app.line();
    assert_eq!(first["snapshot"]["online"], false);
    // Oxy reached (a token minted): a new snapshot.
    socket.machine.platform.token(
        me(),
        &Attestation::OwnPlatform,
        crate::platform::TokenReason::Boot,
    );
    let second = app.line();
    assert_eq!(second["snapshot"]["online"], true);
    assert!(
        second["snapshot"]["revision"].as_u64().unwrap()
            > first["snapshot"]["revision"].as_u64().unwrap()
    );
    // Signed out by Oxy: signed out.
    socket.machine.api().revoke();
    socket.machine.platform.token(
        me(),
        &Attestation::OwnPlatform,
        crate::platform::TokenReason::Response401,
    );
    let third = app.line();
    assert_eq!(third["snapshot"]["status"], "signed_out");
    assert_eq!(third["snapshot"]["account"], serde_json::Value::Null);
    app.end();
}

#[test]
fn only_settings_registered_as_a_system_application_removes_the_account() {
    let socket = service(FakeApi::default());
    let registrar = Registrar::connect(&socket.path).unwrap();
    for (id, system) in [("so.oxy.dropsea", true), ("so.oxy.settings", false)] {
        let mut app = Application::start(&socket, serde_json::json!([{"send": {"remove": {}}}]));
        registrar
            .register(id, system, app.pidfd().as_fd())
            .unwrap()
            .unwrap();
        app.go();
        assert_eq!(app.line(), not_permitted());
        app.end();
    }
    assert!(socket.machine.stored().credential.is_some());
    let mut settings = Application::start(&socket, serde_json::json!([{"send": {"remove": {}}}]));
    registrar
        .register("so.oxy.settings", true, settings.pidfd().as_fd())
        .unwrap()
        .unwrap();
    settings.go();
    assert_eq!(settings.line(), serde_json::json!("removed"));
    settings.end();
    wait_for("the account forgotten", || {
        socket
            .machine
            .platform
            .setup()
            .store()
            .users()
            .unwrap()
            .is_empty()
    });
    assert!(socket.machine.users().names().is_empty());
    assert!(socket.machine.api().calls().contains(&Call::SignOut));
    assert_eq!(
        socket.machine.platform.snapshot(me()).status,
        Status::SignedOut
    );
}

#[test]
fn the_registrar_checks_its_own_accounts_password() {
    let socket = service(FakeApi::default());
    assert_eq!(
        crate::session::check(&socket.path, Secret::new(PASSWORD), Local::Accepted).unwrap(),
        Checked::ACCEPTED
    );
    assert_eq!(
        crate::session::check(&socket.path, Secret::new("not it"), Local::Rejected).unwrap(),
        Checked::REJECTED
    );
    socket.machine.platform.settle();
}

#[test]
fn a_program_that_is_not_the_session_program_registers_nothing() {
    let machine = machine_as(FakeApi::default(), PASSWORD, Some(me()));
    let path = machine.dir.path().join("session");
    let listener = crate::ipc::Listener::bind(&path).unwrap();
    // The session program is another file than the test's.
    let sessions = Arc::new(Sessions::new(
        machine.platform.clone(),
        Registrars {
            program: PathBuf::from("/bin/sh"),
            owner: 0,
        },
    ));
    let serving = sessions.clone();
    std::thread::spawn(move || {
        let connection = listener.accept().unwrap();
        let kind = serving.classify(&connection);
        serving.serve(&connection, kind, |_| {});
    });
    let registrar = Registrar::connect(&path).unwrap();
    let child = std::process::Command::new("sleep")
        .arg("5")
        .spawn()
        .unwrap();
    let pidfd = pidfd::open(child.id()).unwrap();
    assert_eq!(
        registrar
            .register("so.oxy.dropsea", false, pidfd.as_fd())
            .unwrap(),
        Err(Reason::NotPermitted)
    );
    assert!(sessions.registered().is_empty());
    let mut child = child;
    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn the_session_registers_on_a_thread_and_never_waits_for_the_service() {
    // No service: registering returns at once, and nothing breaks.
    let dir = TempDir::new("session-none");
    let lonely = RegistrarThread::spawn(dir.path().join("none"), |_| {}).unwrap();
    let mut child = std::process::Command::new("sleep")
        .arg("5")
        .spawn()
        .unwrap();
    let started = Instant::now();
    lonely.register("so.oxy.dropsea", false, pidfd::open(child.id()).unwrap());
    assert!(started.elapsed() < Duration::from_millis(100));
    let _ = child.kill();
    let _ = child.wait();
    // With the service: the launch is registered, and forgotten as it exits.
    let socket = service(FakeApi::default());
    let registrar = RegistrarThread::spawn(socket.path.clone(), |said| {
        panic!("the registrar said: {said}")
    })
    .unwrap();
    let mut child = std::process::Command::new("sleep")
        .arg("5")
        .spawn()
        .unwrap();
    registrar.register("so.oxy.dropsea", false, pidfd::open(child.id()).unwrap());
    wait_for("the launch registered", || {
        socket.sessions.registered().len() == 1
    });
    let _ = child.kill();
    let _ = child.wait();
    wait_for("the exited launch forgotten", || {
        socket.sessions.registered().is_empty()
    });
}

#[test]
fn the_wire_format_is_the_documented_one() {
    let register: Request =
        serde_json::from_str(r#"{"register":{"desktopEntryId":"so.oxy.dropsea","system":true}}"#)
            .unwrap();
    assert!(matches!(register, Request::Register { system: true, .. }));
    for (text, expected) in [
        (r#""snapshot""#, "Snapshot"),
        (r#""subscribe""#, "Subscribe"),
        (r#"{"remove":{}}"#, "Remove"),
        (r#"{"token":{"reason":"response-401"}}"#, "Response401"),
        (r#"{"check":{"password":"p","local":"accepted"}}"#, "Check"),
    ] {
        let request: Request = serde_json::from_str(text).unwrap();
        assert!(format!("{request:?}").contains(expected), "{text}");
    }
    assert!(!format!(
        "{:?}",
        serde_json::from_str::<Request>(r#"{"check":{"password":"hunter2","local":"accepted"}}"#)
            .unwrap()
    )
    .contains("hunter2"));
    assert_eq!(
        serde_json::to_string(&Answer::Registered).unwrap(),
        r#""registered""#
    );
    assert_eq!(
        serde_json::to_string(&Answer::Removed).unwrap(),
        r#""removed""#
    );
    assert_eq!(
        serde_json::to_string(&Answer::Token(None)).unwrap(),
        r#"{"token":null}"#
    );
    assert_eq!(
        serde_json::to_string(&Answer::refused(Reason::NotPermitted)).unwrap(),
        r#"{"refused":{"reason":"notPermitted"}}"#
    );
    assert_eq!(
        serde_json::to_string(&Answer::Token(Some(crate::platform::Token {
            access_token: Secret::new("t"),
            expires_at: "2026-11-06T12:05:00.000Z".into(),
        })))
        .unwrap(),
        r#"{"token":{"accessToken":"t","expiresAt":"2026-11-06T12:05:00.000Z"}}"#
    );
}
