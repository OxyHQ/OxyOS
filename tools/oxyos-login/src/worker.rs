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
//! On the development path (`--record FILE`), nothing is started: the session's PAM transaction
//! runs (a test configuration's, `--pam-confdir`), the worker writes what it would start in the
//! file, and the session ends at once.

use crate::channel::{Channel, FromWorker, ToWorker};
use oxyos_auth::login::Login;
use oxyos_auth::user::User;
use oxyos_auth::Outcome;
use std::ffi::OsString;
use std::io::Write;
use std::os::fd::AsFd;
use std::path::PathBuf;
use std::process::Child;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How long a session's program has to stop once asked, before it is killed.
const STOP_WAIT: Duration = Duration::from_secs(5);
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
    if let Err(error) = login.open_session() {
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
    let env = environment(&user, &login, &options.env);
    eprintln!(
        "[login] {} signed in ({} session on {} terminal {})",
        user.name, options.class, options.seat, options.vt
    );

    let mut child = user
        .spawn(&options.program[0], &options.program[1..], &env)
        .map_err(|error| format!("starting {:?}: {error}", options.program[0]))?;
    let _ = channel.tell(FromWorker::Started(child.id()));
    let status = wait(&mut child, &channel, &stop);
    eprintln!("[login] {}'s session ended ({status})", user.name);
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

/// Waits for the session's program to exit, or the login program to ask it to stop (the channel
/// closed, or `SIGTERM`), then stops it.
fn wait(child: &mut Child, channel: &Channel, stop: &AtomicBool) -> String {
    use rustix::event::{poll, PollFd, PollFlags};
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.to_string(),
            Ok(None) => {}
            Err(error) => return error.to_string(),
        }
        if stop.load(Ordering::Relaxed) {
            return end(child, "asked to stop");
        }
        let mut fds = [PollFd::new(channel, PollFlags::IN)];
        let timeout = rustix::time::Timespec {
            tv_sec: 0,
            tv_nsec: 200_000_000,
        };
        if poll(&mut fds, Some(&timeout)).is_ok_and(|ready| ready > 0) {
            // Nothing else is said during a session: anything readable is the login program
            // going (or a message out of turn), and ends it.
            let _ = channel.next();
            return end(child, "the login program ended it");
        }
    }
}

/// Asks the program to stop (SIGTERM to its process group), and kills it after [`STOP_WAIT`].
fn end(child: &mut Child, why: &str) -> String {
    eprintln!("[login] stopping the session: {why}");
    if let Some(group) = rustix::process::Pid::from_raw(child.id() as i32) {
        let _ = rustix::process::kill_process_group(group, rustix::process::Signal::TERM);
    }
    let deadline = Instant::now() + STOP_WAIT;
    while Instant::now() < deadline {
        if let Ok(Some(status)) = child.try_wait() {
            return status.to_string();
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = child.kill();
    child
        .wait()
        .map_or_else(|error| error.to_string(), |status| status.to_string())
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
