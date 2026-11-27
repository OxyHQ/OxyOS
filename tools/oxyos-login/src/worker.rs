//! A worker: one PAM transaction and the session it opens, as a login manager's session worker
//! (GDM's `gdm-session-worker`, LightDM's session child), each in a process of its own started by
//! the login program (`oxyos-login --worker`, root). `pam_systemd` makes the process that opens a
//! session its leader and moves it into the session's scope, so the login program never opens one
//! itself; and a PAM module's state lives and dies with one transaction.
//!
//! - **A person's worker** (`--class user`) waits for the password, authenticates it and checks the
//!   account (`oxyos-login`), and says what came of it. Rejected, it ends. Accepted, it waits for
//!   the login program to say the login screen has ended, then opens the session.
//! - **The login screen's worker** (`--class greeter`) checks its account (`oxyos-login-greeter`)
//!   and opens its session at once.
//!
//! Opening: the terminal is shown, credentials established and the session opened, with logind's
//! class, type, seat and terminal in PAM's environment (`pam_systemd` reads them). The program
//! then starts as the user, from its home, in a session of processes of its own, with a clean
//! environment: the user's own variables, what the modules set (`XDG_SESSION_ID`,
//! `XDG_RUNTIME_DIR`, the locale), and the login program's few. When it exits, or the login
//! program asks (closing the channel, or `SIGTERM`), the program is stopped, the session closed
//! and the credentials deleted.
//!
//! **Its heartbeat** (docs/adr/0065-the-session-ending-because-of-a-problem.mdx, decision 1): the
//! worker binds a datagram socket in the person's runtime directory, theirs alone, which the
//! program is told (`OXYOS_HEARTBEAT`, `oxyos_seat::heartbeat`) and says it is alive on as its
//! turns end. Once it has said it, silence for `heartbeat::HANG` is a
//! hang, as a Mac's watchdogd holds a window server that stops checking in: the program is sent
//! `SIGABRT` (its core and report, as a crash's) and, if it has not ended [`ABORT_WAIT`] later,
//! its process group `SIGKILL`. Its logind session then closes, the login screen comes back and
//! the power key's inhibitor goes with the process: a hung session is never a hung machine.
//! Before it ends, the worker tells the login program how the session ended
//! ([`FromWorker::Ended`]).
//!
//! On the development path (`--record FILE`), nothing is started: the session's PAM transaction
//! runs (a test configuration's, `--pam-confdir`), the worker writes what it would start in the
//! file, and the session ends at once.

use crate::channel::{Channel, End, FromWorker, ToWorker};
use oxyos_auth::login::Login;
use oxyos_auth::user::User;
use oxyos_auth::Outcome;
#[cfg(test)]
use oxyos_seat::heartbeat::Heartbeat;
use oxyos_seat::heartbeat::{self, Watch};
use std::ffi::OsString;
use std::io::Write;
use std::os::fd::AsFd;
#[cfg(test)]
use std::os::unix::net::UnixDatagram;
use std::path::PathBuf;
use std::process::Child;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How long a session's program has to stop once asked, before it is killed.
const STOP_WAIT: Duration = Duration::from_secs(5);
/// How long a hung session's program has to write its core once sent `SIGABRT`.
const ABORT_WAIT: Duration = Duration::from_secs(3);
/// What a session's programs find on their path.
const PATH: &str = "/usr/local/bin:/usr/bin:/bin";

#[derive(Debug, Default)]
pub struct Options {
    pub service: String,
    pub user: String,
    /// logind's class: `greeter` (the login screen's) or `user`.
    pub class: String,
    pub seat: String,
    pub vt: u16,
    pub confdir: Option<PathBuf>,
    pub record: Option<PathBuf>,
    pub env: Vec<(String, String)>,
    pub program: Vec<OsString>,
}

impl Options {
    pub fn parse(mut args: impl Iterator<Item = OsString>) -> Result<Self, String> {
        let mut options = Options {
            seat: "seat0".into(),
            vt: 1,
            ..Options::default()
        };
        while let Some(arg) = args.next() {
            let flag = arg.to_string_lossy().into_owned();
            if flag == "--" {
                options.program = args.collect();
                break;
            }
            let value = args
                .next()
                .and_then(|value| value.into_string().ok())
                .ok_or_else(|| format!("{flag} needs a value"))?;
            match flag.as_str() {
                "--service" => options.service = value,
                "--user" => options.user = value,
                "--class" => options.class = value,
                "--seat" => options.seat = value,
                "--vt" => options.vt = value.parse().map_err(|_| "--vt: a number")?,
                "--pam-confdir" => options.confdir = Some(value.into()),
                "--record" => options.record = Some(value.into()),
                "--env" => {
                    let (name, value) = value.split_once('=').ok_or("--env: NAME=VALUE")?;
                    options.env.push((name.into(), value.into()));
                }
                _ => return Err(format!("the worker does not take {flag}")),
            }
        }
        if options.service.is_empty()
            || options.user.is_empty()
            || !["greeter", "user"].contains(&options.class.as_str())
            || options.program.is_empty()
        {
            return Err(
                "a worker needs --service, --user, --class greeter|user and a program".into(),
            );
        }
        Ok(options)
    }
}

/// The worker's whole life; its exit status says only whether it failed.
pub fn run(options: Options) -> Result<(), String> {
    let stop = Arc::new(AtomicBool::new(false));
    for signal in [signal_hook::consts::SIGTERM, signal_hook::consts::SIGINT] {
        signal_hook::flag::register(signal, stop.clone()).map_err(|error| error.to_string())?;
    }
    // Its channel is its standard input; what it starts reads /dev/null instead.
    let channel = Channel::from_fd(
        std::io::stdin()
            .as_fd()
            .try_clone_to_owned()
            .map_err(|error| format!("the channel: {error}"))?,
    );
    let mut login = Login::start(&options.service, &options.user, options.confdir.as_deref())
        .map_err(|error| error.to_string())?;
    let tty = format!("/dev/tty{}", options.vt);
    let prepared = login.set_tty(&tty).and_then(|()| {
        for (name, value) in [
            ("XDG_SESSION_CLASS", options.class.as_str()),
            // OxyOS draws on the terminal's display itself: logind's types name display
            // servers OxyOS has none of, and `tty` is what its sessions were verified with.
            ("XDG_SESSION_TYPE", "tty"),
            ("XDG_SEAT", options.seat.as_str()),
            ("XDG_VTNR", &options.vt.to_string()),
        ] {
            login.set_env(name, value)?;
        }
        Ok(())
    });
    if let Err(error) = prepared {
        let _ = channel.tell(FromWorker::Failed);
        return Err(error.to_string());
    }

    if options.class == "user" {
        let secret = match channel.next() {
            Ok(ToWorker::Password(secret)) => secret,
            Ok(ToWorker::Open) => return Err("asked to open before a password".into()),
            // The login program went, or abandoned the check.
            Err(_) => return Ok(()),
        };
        let (told, accepted) = match login.authenticate(secret) {
            Ok(Outcome::Accepted) => (FromWorker::Accepted, true),
            Ok(Outcome::Rejected) => (FromWorker::Rejected, false),
            Err(error) => {
                eprintln!("[login] {}: {error}", options.user);
                (FromWorker::Failed, false)
            }
        };
        channel
            .tell(told)
            .map_err(|error| format!("the channel: {error}"))?;
        if !accepted {
            return Ok(());
        }
        // The login screen ends first: the session opens once it has the seat to itself.
        match channel.next() {
            Ok(ToWorker::Open) => {}
            Ok(ToWorker::Password(_)) => return Err("a second password".into()),
            Err(_) => return Ok(()),
        }
    } else {
        match login.check_account() {
            Ok(Outcome::Accepted) => {}
            outcome => {
                let _ = channel.tell(FromWorker::Failed);
                return Err(format!("{} may not sign in: {outcome:?}", options.user));
            }
        }
    }

    // The session's terminal shown, so logind makes the session the seat's active one. Only
    // root can, and only a real login does.
    if options.record.is_none() {
        if let Err(error) = oxyos_vt::show(options.vt) {
            eprintln!("[login] showing terminal {}: {error}", options.vt);
        }
    }
    // The login screen's session is never authenticated, so it establishes no credentials.
    let opened = if options.class == "greeter" {
        login.open_greeter_session()
    } else {
        login.open_session()
    };
    if let Err(error) = opened {
        let _ = channel.tell(FromWorker::Failed);
        return Err(format!("{}: {error}", options.user));
    }
    if let Some(record) = &options.record {
        // The development path: what would start (its user need not exist here), and the
        // session ends at once.
        let program: Vec<String> = options
            .program
            .iter()
            .map(|part| part.to_string_lossy().into_owned())
            .collect();
        let mut names: Vec<String> = login
            .environment()
            .into_iter()
            .chain(options.env.iter().cloned())
            .map(|(name, _)| name)
            .collect();
        names.sort();
        names.dedup();
        let line = format!(
            "session would start for {} ({} session): {} [{}]\n",
            options.user,
            options.class,
            program.join(" "),
            names.join(" ")
        );
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(record)
            .and_then(|mut file| file.write_all(line.as_bytes()))
            .map_err(|error| format!("{}: {error}", record.display()))?;
        eprintln!("[login] {line}");
        let _ = channel.tell(FromWorker::Started(0));
        return login.close_session().map_err(|error| error.to_string());
    }

    let user = match User::named(&options.user) {
        Ok(Some(user)) => user,
        Ok(None) => return Err(format!("{} has no account entry", options.user)),
        Err(error) => return Err(format!("{}: {error}", options.user)),
    };
    let mut env = environment(&user, &login, &options.env);
    eprintln!(
        "[login] {} signed in ({} session on {} terminal {})",
        user.name, options.class, options.seat, options.vt
    );

    // Its heartbeat's socket, in the person's runtime directory (pam_systemd's), theirs alone.
    let heartbeat_path = env
        .iter()
        .find(|(name, _)| name == "XDG_RUNTIME_DIR")
        .map(|(_, dir)| PathBuf::from(dir).join(format!("oxyos-heartbeat-{}", std::process::id())));
    let beats = heartbeat_path
        .as_ref()
        .and_then(|path| match heartbeat::listen(path, user.uid) {
            Ok(socket) => Some(socket),
            Err(error) => {
                eprintln!(
                    "[login] the session's heartbeat, {}: {error}: it is not watched",
                    path.display()
                );
                None
            }
        });
    if let (Some(path), Some(_)) = (&heartbeat_path, &beats) {
        env.push((heartbeat::VARIABLE.into(), path.display().to_string()));
    }
    let started = Instant::now();
    let mut child = user
        .spawn(&options.program[0], &options.program[1..], &env)
        .map_err(|error| format!("starting {:?}: {error}", options.program[0]))?;
    let _ = channel.tell(FromWorker::Started(child.id()));
    let ended = wait(
        &mut child,
        &channel,
        &stop,
        beats.as_ref(),
        Watch::default(),
    );
    let ran = u32::try_from(started.elapsed().as_secs()).unwrap_or(u32::MAX);
    eprintln!(
        "[login] {}'s session ended ({}, after {ran} s)",
        user.name,
        ended.word()
    );
    let _ = channel.tell(FromWorker::Ended(ended, ran));
    login.close_session().map_err(|error| error.to_string())
}

/// The session's environment: the user's own, the modules', and the login program's.
fn environment(user: &User, login: &Login, extra: &[(String, String)]) -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = vec![
        ("HOME".into(), user.home.to_string_lossy().into_owned()),
        ("USER".into(), user.name.clone()),
        ("LOGNAME".into(), user.name.clone()),
        ("SHELL".into(), user.shell.to_string_lossy().into_owned()),
        ("PATH".into(), PATH.into()),
    ];
    for (name, value) in login.environment().into_iter().chain(extra.iter().cloned()) {
        env.retain(|(kept, _)| *kept != name);
        env.push((name, value));
    }
    env
}

/// Waits for the session's program to exit, the login program to ask it to stop (the channel
/// closed, or `SIGTERM`), or its heartbeat to stop (`beats`, watched by `watch`), then stops it:
/// how it ended.
fn wait(
    child: &mut Child,
    channel: &Channel,
    stop: &AtomicBool,
    beats: Option<&std::os::unix::net::UnixDatagram>,
    mut watch: Watch,
) -> End {
    use rustix::event::{poll, PollFd, PollFlags};
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return end_of(status),
            Ok(None) => {}
            Err(error) => {
                eprintln!("[login] waiting for the session: {error}");
                end(child, "it cannot be waited for");
                return End::Asked;
            }
        }
        if stop.load(Ordering::Relaxed) {
            end(child, "asked to stop");
            return End::Asked;
        }
        if watch.hung(Instant::now()) {
            return hung(child);
        }
        let mut fds = vec![PollFd::new(channel, PollFlags::IN)];
        if let Some(beats) = beats {
            fds.push(PollFd::new(beats, PollFlags::IN));
        }
        let mut wait = Duration::from_millis(200);
        if let Some(until) = watch.until(Instant::now()) {
            wait = wait.min(until + Duration::from_millis(1));
        }
        let timeout = rustix::time::Timespec {
            tv_sec: 0,
            tv_nsec: wait.subsec_nanos().into(),
        };
        if poll(&mut fds, Some(&timeout)).is_err() {
            continue;
        }
        let said = fds
            .get(1)
            .is_some_and(|fd| fd.revents().contains(PollFlags::IN));
        if fds[0].revents().intersects(PollFlags::IN | PollFlags::HUP) {
            // Nothing else is said during a session: anything readable is the login program
            // going (or a message out of turn), and ends it.
            let _ = channel.next();
            end(child, "the login program ended it");
            return End::Asked;
        }
        if let Some(socket) = beats.filter(|_| said) {
            match heartbeat::read_beats(socket) {
                Ok(bytes) => watch.heard(&bytes, Instant::now()),
                Err(error) => eprintln!("[login] the session's heartbeat: {error}"),
            }
        }
    }
}

/// How a program that exited ended.
fn end_of(status: std::process::ExitStatus) -> End {
    use std::os::unix::process::ExitStatusExt;
    match (status.code(), status.signal()) {
        (Some(code), _) => End::Exited(u8::try_from(code).unwrap_or(u8::MAX)),
        (None, Some(signal)) => End::Signal(u8::try_from(signal).unwrap_or(u8::MAX)),
        (None, None) => End::Exited(u8::MAX),
    }
}

/// The session stopped answering: `SIGABRT` (and `SIGCONT`, for one that was stopped), so its
/// core and report are written as a crash's; its process group killed if it has not ended
/// [`ABORT_WAIT`] later.
fn hung(child: &mut Child) -> End {
    use rustix::process::{kill_process, kill_process_group, Pid, Signal};
    eprintln!(
        "[login] the session said nothing for {} s: it is hung, and ended",
        heartbeat::HANG.as_secs()
    );
    let Some(pid) = Pid::from_raw(child.id() as i32) else {
        return End::Hung;
    };
    let _ = kill_process(pid, Signal::ABORT);
    let _ = kill_process(pid, Signal::CONT);
    let deadline = Instant::now() + ABORT_WAIT;
    while Instant::now() < deadline {
        if let Ok(Some(_)) = child.try_wait() {
            return End::Hung;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    eprintln!("[login] the hung session did not end on SIGABRT: killed");
    let _ = kill_process_group(pid, Signal::KILL);
    let _ = child.kill();
    let _ = child.wait();
    End::Hung
}

/// Asks the program to stop (SIGTERM to its process group), and kills it after [`STOP_WAIT`].
fn end(child: &mut Child, why: &str) {
    eprintln!("[login] stopping the session: {why}");
    if let Some(group) = rustix::process::Pid::from_raw(child.id() as i32) {
        let _ = rustix::process::kill_process_group(group, rustix::process::Signal::TERM);
    }
    let deadline = Instant::now() + STOP_WAIT;
    while Instant::now() < deadline {
        if let Ok(Some(_)) = child.try_wait() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(text: &str) -> impl Iterator<Item = OsString> {
        text.split(' ')
            .map(OsString::from)
            .collect::<Vec<_>>()
            .into_iter()
    }

    /// A stand-in session: `script` run by sh in a process group of its own, and the worker's
    /// heartbeat socket with the session's end connected to it.
    fn stand_in(name: &str, script: &str) -> (Child, UnixDatagram, Heartbeat, PathBuf) {
        use std::os::unix::process::CommandExt;
        let path = std::env::temp_dir().join(format!("oxyos-worker-{name}-{}", std::process::id()));
        let socket = heartbeat::listen(&path, rustix::process::geteuid().as_raw()).unwrap();
        let session = Heartbeat::connect(&path).unwrap();
        let child = std::process::Command::new("sh")
            .arg("-c")
            .arg(script)
            .process_group(0)
            .spawn()
            .unwrap();
        (child, socket, session, path)
    }

    /// The stand-in's beats, `count` of them `every` apart, from a thread of the test's.
    fn beating(mut session: Heartbeat, count: u32, every: Duration) -> std::thread::JoinHandle<()> {
        std::thread::spawn(move || {
            for _ in 0..count {
                session.beat(Instant::now());
                std::thread::sleep(every);
            }
        })
    }

    /// A second's silence is a hang here: the session beats at most every half second.
    fn quick() -> Watch {
        Watch::with_limits(Duration::from_secs(1), Duration::from_secs(3))
    }

    fn channel() -> (Channel, Channel) {
        let (ours, theirs) = crate::channel::pair().unwrap();
        (ours, Channel::from_fd(theirs))
    }

    #[test]
    fn a_session_that_stops_answering_is_aborted_and_said_hung() {
        // It beats, then goes on running without a word (a deadlock, a busy loop).
        let (mut child, socket, session, path) = stand_in("silent", "exec sleep 600");
        let beats = beating(session, 2, Duration::from_millis(500));
        let (_login, worker) = channel();
        let started = Instant::now();
        let ended = wait(
            &mut child,
            &worker,
            &AtomicBool::new(false),
            Some(&socket),
            quick(),
        );
        assert_eq!(ended, End::Hung);
        assert!(started.elapsed() >= Duration::from_millis(1000));
        // Ended by its SIGABRT (sleep has no handler), well before the kill.
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(child.wait().unwrap().signal(), Some(libc::SIGABRT));
        assert!(started.elapsed() < ABORT_WAIT + Duration::from_secs(2));
        beats.join().unwrap();
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_hung_session_that_ignores_sigabrt_is_killed() {
        let (mut child, socket, session, path) =
            stand_in("stubborn", "trap '' ABRT; while :; do sleep 0.05; done");
        let beats = beating(session, 1, Duration::ZERO);
        let (_login, worker) = channel();
        let started = Instant::now();
        let ended = wait(
            &mut child,
            &worker,
            &AtomicBool::new(false),
            Some(&socket),
            quick(),
        );
        assert_eq!(ended, End::Hung);
        assert!(started.elapsed() >= ABORT_WAIT);
        assert!(child.try_wait().unwrap().is_some(), "killed and reaped");
        beats.join().unwrap();
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_stopped_session_is_ended_too() {
        let (mut child, socket, session, path) =
            stand_in("stopped", "kill -STOP $$; exec sleep 600");
        let beats = beating(session, 1, Duration::ZERO);
        let (_login, worker) = channel();
        assert_eq!(
            wait(
                &mut child,
                &worker,
                &AtomicBool::new(false),
                Some(&socket),
                quick()
            ),
            End::Hung
        );
        beats.join().unwrap();
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_long_stall_with_its_beat_after_it_is_no_hang() {
        // A modeset held on the turn, shorter than the limit, between beats: never a hang.
        let (mut child, socket, session, path) = stand_in("stalls", "sleep 3.3; exit 0");
        let beats = beating(session, 6, Duration::from_millis(600));
        let (_login, worker) = channel();
        assert_eq!(
            wait(
                &mut child,
                &worker,
                &AtomicBool::new(false),
                Some(&socket),
                quick()
            ),
            End::Exited(0)
        );
        beats.join().unwrap();
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_session_that_never_beats_is_never_held_hung_and_its_crash_is_said() {
        // A session started from elsewhere (no heartbeat): its start has its own deadlines.
        let (mut child, socket, _session, path) = stand_in("crashes", "sleep 1; kill -SEGV $$");
        let (_login, worker) = channel();
        assert_eq!(
            wait(
                &mut child,
                &worker,
                &AtomicBool::new(false),
                Some(&socket),
                quick()
            ),
            End::Signal(libc::SIGSEGV as u8)
        );
        let (mut child, socket, _session, path2) = stand_in("fails", "exit 1");
        assert_eq!(
            wait(
                &mut child,
                &worker,
                &AtomicBool::new(false),
                Some(&socket),
                quick()
            ),
            End::Exited(1)
        );
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(path2);
    }

    #[test]
    fn a_session_the_login_program_ends_was_asked_to_end() {
        let (mut child, socket, session, path) = stand_in("asked", "exec sleep 600");
        let beats = beating(session, 3, Duration::from_millis(100));
        let (login, worker) = channel();
        drop(login);
        assert_eq!(
            wait(
                &mut child,
                &worker,
                &AtomicBool::new(false),
                Some(&socket),
                quick()
            ),
            End::Asked
        );
        beats.join().unwrap();
        let (mut child, socket, _session, path2) = stand_in("stopping", "exec sleep 600");
        let (_login, worker) = channel();
        assert_eq!(
            wait(
                &mut child,
                &worker,
                &AtomicBool::new(true),
                Some(&socket),
                quick()
            ),
            End::Asked
        );
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(path2);
    }

    #[test]
    fn a_worker_is_told_its_transaction_and_program() {
        let options = Options::parse(args(
            "--service oxyos-login --user oxy-a --class user --vt 2 --env A=b=c -- /usr/bin/oxyos-session --shell x",
        ))
        .unwrap();
        assert_eq!(options.service, "oxyos-login");
        assert_eq!(options.user, "oxy-a");
        assert_eq!(options.seat, "seat0");
        assert_eq!(options.vt, 2);
        assert_eq!(options.env, [("A".to_string(), "b=c".to_string())]);
        assert_eq!(options.program.len(), 3);
        for wrong in [
            "--service s --user u --class root -- p",
            "--service s --user u --class user",
            "--user u --class user -- p",
            "--service s --user u --class user --bogus x -- p",
        ] {
            assert!(Options::parse(args(wrong)).is_err(), "{wrong}");
        }
    }
}
