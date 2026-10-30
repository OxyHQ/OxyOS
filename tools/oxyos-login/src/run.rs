//! The login program doing what its state machine (`manager.rs`) decides: the login screen's
//! worker and the person's worker as processes, the login screen's socket, the accounts' service,
//! the service manager told once the login screen is on screen.

use crate::channel::{self, Channel, FromWorker};
use crate::manager::{Action, Checked, Manager};
use oxyos_auth::greeter::{self, Answer, ToLogin, Verdict};
use oxyos_auth::Secret;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub struct Config {
    /// The development path: no login screen of its own (the development compositor connects as
    /// the login screen, as this program's user), PAM from `confdir`, and sessions recorded in
    /// `record`, never started.
    pub dev: bool,
    pub confdir: Option<PathBuf>,
    pub record: Option<PathBuf>,
    /// The login screen's user (`oxyos-login`).
    pub greeter_user: String,
    /// The login screen's socket.
    pub socket: PathBuf,
    /// The accounts' service's socket.
    pub accountd: PathBuf,
    pub seat: String,
    pub vt: u16,
    /// The session program (`oxyos-session`), which runs both the login screen (`--greeter`) and
    /// the person's session.
    pub session: PathBuf,
    /// The Bloom host and the bundle with the login screen (`OxyOSLogin`) and the shell
    /// (`OxyOSShell`).
    pub host: PathBuf,
    pub bundle: PathBuf,
}

struct Worker {
    child: Child,
    channel: Channel,
    /// The program it started, once it has.
    program: Option<u32>,
}

impl Worker {
    fn spawn(args: Vec<OsString>) -> std::io::Result<Self> {
        let (channel, theirs) = channel::pair()?;
        let mut command = Command::new(std::env::current_exe()?);
        command
            .arg("--worker")
            .args(args)
            .stdin(Stdio::from(theirs));
        Ok(Self {
            child: command.spawn()?,
            channel,
            program: None,
        })
    }

    fn signal(&self, signal: rustix::process::Signal) {
        if let Some(pid) = rustix::process::Pid::from_raw(self.child.id() as i32) {
            let _ = rustix::process::kill_process(pid, signal);
        }
    }

    /// Kills the worker and the program it started.
    fn kill(&mut self) {
        if let Some(pid) = self
            .program
            .and_then(|pid| rustix::process::Pid::from_raw(pid as i32))
        {
            let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
        }
        let _ = self.child.kill();
    }
}

/// What the login screen and sessions are given of the login program's own environment (the
/// unit's `Environment=`): the solar wallpaper's folder and the screen reader.
const PASSED_ON: [&str; 2] = ["OXYOS_WALLPAPER", "OXYOS_READER"];

/// `--flag value` pairs and the program after `--`.
fn worker_args(
    config: &Config,
    service: &str,
    user: &str,
    class: &str,
    env: &[(&str, String)],
    program: Vec<OsString>,
) -> Vec<OsString> {
    let mut args: Vec<OsString> = [
        ("--service", service.to_owned()),
        ("--user", user.to_owned()),
        ("--class", class.to_owned()),
        ("--seat", config.seat.clone()),
        ("--vt", config.vt.to_string()),
    ]
    .into_iter()
    .flat_map(|(flag, value)| [flag.into(), value.into()])
    .collect();
    if let Some(record) = &config.record {
        args.extend(["--record".into(), record.clone().into_os_string()]);
    }
    if let Some(dir) = &config.confdir {
        args.extend(["--pam-confdir".into(), dir.clone().into_os_string()]);
    }
    // The session's settings the unit gives the login program, passed on; nothing else of its
    // environment is.
    let passed = PASSED_ON
        .iter()
        .filter_map(|name| Some((*name, std::env::var(name).ok()?)));
    for (name, value) in env.iter().cloned().chain(passed) {
        args.extend(["--env".into(), format!("{name}={value}").into()]);
    }
    args.push("--".into());
    args.extend(program);
    args
}

struct Login {
    config: Config,
    manager: Manager,
    greeter_uid: u32,
    listener: greeter::Listener,
    connection: Option<greeter::Connection>,
    greeter: Option<Worker>,
    /// When the login screen is to start.
    greeter_due: Option<Instant>,
    person: Option<Worker>,
    /// Workers whose checks were abandoned, until they have ended.
    abandoned: Vec<Child>,
    /// The password the login screen sent, until a worker takes it.
    pending: Option<Secret>,
    notify: Option<oxyos_seat::notify::Notify>,
}

pub fn run(config: Config, notify: Option<oxyos_seat::notify::Notify>) -> Result<(), String> {
    let greeter_uid = if config.dev {
        rustix::process::getuid().as_raw()
    } else {
        if !rustix::process::geteuid().is_root() {
            return Err("it runs as root: it checks passwords and opens sessions".into());
        }
        oxyos_auth::user::User::named(&config.greeter_user)
            .map_err(|error| format!("{}: {error}", config.greeter_user))?
            .ok_or_else(|| {
                format!(
                    "no user {} (sysusers.d/oxyos-login.conf)",
                    config.greeter_user
                )
            })?
            .uid
    };
    let listener = greeter::Listener::bind(&config.socket, greeter_uid)
        .map_err(|error| format!("{}: {error}", config.socket.display()))?;
    let stop = Arc::new(AtomicBool::new(false));
    for signal in [signal_hook::consts::SIGTERM, signal_hook::consts::SIGINT] {
        signal_hook::flag::register(signal, stop.clone()).map_err(|error| error.to_string())?;
    }
    eprintln!(
        "[login] the login screen on {} terminal {}, its socket {}{}",
        config.seat,
        config.vt,
        config.socket.display(),
        if config.dev {
            " (development: sessions recorded, never started)"
        } else {
            ""
        }
    );
    let (manager, actions) = Manager::start(Instant::now());
    let mut login = Login {
        config,
        manager,
        greeter_uid,
        listener,
        connection: None,
        greeter: None,
        greeter_due: None,
        person: None,
        abandoned: Vec::new(),
        pending: None,
        notify,
    };
    login.execute(actions);
    while !stop.load(Ordering::Relaxed) {
        login.turn();
    }
    login.shut_down();
    Ok(())
}

impl Login {
    fn execute(&mut self, actions: Vec<Action>) {
        for action in actions {
            self.act(action);
        }
    }

    fn act(&mut self, action: Action) {
        let now = Instant::now();
        match action {
            Action::StartGreeter { after } => self.greeter_due = Some(now + after),
            Action::StopGreeter if self.config.dev => self.greeter_gone(),
            Action::StopGreeter => {
                eprintln!("[login] the login screen has not ended: stopping it");
                if let Some(greeter) = &self.greeter {
                    greeter.signal(rustix::process::Signal::TERM);
                }
            }
            Action::KillGreeter if self.config.dev => self.greeter_gone(),
            Action::KillGreeter => {
                eprintln!("[login] the login screen has not stopped: killing it");
                if let Some(greeter) = &mut self.greeter {
                    greeter.kill();
                }
            }
            Action::Answer { verdict, wait_s } => {
                if let Some(connection) = &self.connection {
                    if let Err(error) = connection.answer(Answer { verdict, wait_s }) {
                        eprintln!("[login] answering the login screen: {error}");
                    }
                }
                // Signed in: on the development path the login screen goes on, as the
                // development compositor does; its part is over.
                if verdict == Verdict::Accepted && self.config.dev {
                    self.greeter_gone();
                }
            }
            Action::Check { account } => {
                let secret = self
                    .pending
                    .take()
                    .unwrap_or_else(|| Secret::new(Vec::new()));
                let started = self.start_person(&account).and_then(|worker| {
                    worker.channel.send_password(secret)?;
                    Ok(worker)
                });
                match started {
                    Ok(worker) => self.person = Some(worker),
                    Err(error) => {
                        eprintln!("[login] checking {account}'s password: {error}");
                        let actions = self.manager.checked(Checked::Failed, now);
                        self.execute(actions);
                    }
                }
            }
            Action::AbandonCheck => {
                // Its channel closed and asked to stop, it ends unanswered; reaped later.
                if let Some(worker) = self.person.take() {
                    worker.signal(rustix::process::Signal::TERM);
                    self.abandoned.push(worker.child);
                }
            }
            Action::OpenSession => {
                let opened = self
                    .person
                    .as_ref()
                    .map(|worker| worker.channel.send_open());
                if !matches!(opened, Some(Ok(()))) {
                    eprintln!("[login] the session's worker is gone");
                }
            }
        }
    }

    /// On the development path, the login screen's part ends when it is told: its connection is
    /// let go, as a login screen's process would be.
    fn greeter_gone(&mut self) {
        self.connection = None;
        let actions = self.manager.greeter_ended(Instant::now());
        self.execute(actions);
    }

    fn start_person(&self, account: &str) -> std::io::Result<Worker> {
        let config = &self.config;
        let shell = format!(
            "{},{},OxyOSShell",
            config.host.display(),
            config.bundle.display()
        );
        let args = worker_args(
            config,
            "oxyos-login",
            account,
            "user",
            &[
                ("OXYOS_BLOOM_HOST", config.host.display().to_string()),
                ("OXYOS_BLOOM_BUNDLE", config.bundle.display().to_string()),
            ],
            vec![
                config.session.clone().into(),
                "--shell".into(),
                shell.into(),
            ],
        );
        Worker::spawn(args)
    }

    fn start_greeter(&mut self) {
        let config = &self.config;
        let shell = format!(
            "{},{},OxyOSLogin",
            config.host.display(),
            config.bundle.display()
        );
        let args = worker_args(
            config,
            "oxyos-login-greeter",
            &config.greeter_user,
            "greeter",
            &[
                ("OXYOS_LOGIN_SOCKET", config.socket.display().to_string()),
                (
                    "OXYOS_ACCOUNTD_SOCKET",
                    config.accountd.display().to_string(),
                ),
            ],
            vec![
                config.session.clone().into(),
                "--greeter".into(),
                "--shell".into(),
                shell.into(),
            ],
        );
        match Worker::spawn(args) {
            Ok(worker) => self.greeter = Some(worker),
            Err(error) => {
                eprintln!("[login] starting the login screen: {error}");
                let actions = self.manager.greeter_ended(Instant::now());
                self.execute(actions);
            }
        }
    }

    /// Whether the accounts' service lists `account`. Not reachable, it lists none: nobody signs
    /// in without it (ADR 0020, decision 7).
    fn enrolled(&self, account: &str) -> Option<bool> {
        match oxyos_account::ipc::list(&self.config.accountd) {
            Ok(accounts) => Some(accounts.iter().any(|listed| listed.user == account)),
            Err(error) => {
                eprintln!("[login] the accounts' service: {error}");
                None
            }
        }
    }

    fn turn(&mut self) {
        let now = Instant::now();
        if self.greeter_due.is_some_and(|due| due <= now) {
            self.greeter_due = None;
            if !self.config.dev {
                self.start_greeter();
            }
        }
        // The login screen connecting: the newest connection is the one listened to.
        match self.listener.accept(self.greeter_uid) {
            Ok(Some(connection)) => self.connection = Some(connection),
            Ok(None) => {}
            Err(error) => eprintln!("[login] the login screen's socket: {error}"),
        }
        self.hear_greeter(now);
        self.hear_workers(now);
        let actions = self.manager.tick(now);
        self.execute(actions);
        self.wait();
    }

    fn hear_greeter(&mut self, now: Instant) {
        let Some(connection) = &self.connection else {
            return;
        };
        match connection.receive() {
            Ok(None) => {}
            Ok(Some(ToLogin::Shown)) => {
                if let Some(notify) = self.notify.take() {
                    match notify.ready() {
                        Ok(()) => eprintln!("[login] on screen: the service manager is told"),
                        Err(error) => eprintln!("[login] telling the service manager: {error}"),
                    }
                }
            }
            Ok(Some(ToLogin::Attempt { account, secret })) => {
                let actions = match self.enrolled(&account) {
                    Some(enrolled) => {
                        self.pending = Some(secret);
                        let actions = self.manager.attempt(&account, enrolled, now);
                        if !actions
                            .iter()
                            .any(|action| matches!(action, Action::Check { .. }))
                        {
                            self.pending = None;
                        }
                        actions
                    }
                    None => vec![Action::Answer {
                        verdict: Verdict::Rejected,
                        wait_s: 0,
                    }],
                };
                self.execute(actions);
            }
            Err(error) => {
                if error.kind() != std::io::ErrorKind::UnexpectedEof {
                    eprintln!("[login] the login screen: {error}");
                }
                if self.config.dev {
                    self.greeter_gone();
                } else {
                    self.connection = None;
                }
            }
        }
    }

    fn hear_workers(&mut self, now: Instant) {
        // Abandoned checks, reaped as they end.
        self.abandoned
            .retain_mut(|child| !matches!(child.try_wait(), Ok(Some(_))));
        let mut greeter_ended = None;
        if let Some(greeter) = &mut self.greeter {
            if let Ok(Some(FromWorker::Started(pid))) = greeter.channel.heard() {
                greeter.program = Some(pid);
            }
            if let Ok(Some(status)) = greeter.child.try_wait() {
                greeter_ended = Some(status);
            }
        }
        if let Some(status) = greeter_ended {
            eprintln!("[login] the login screen ended ({status})");
            self.greeter = None;
            self.connection = None;
            let actions = self.manager.greeter_ended(now);
            self.execute(actions);
        }
        let Some(person) = &mut self.person else {
            return;
        };
        let mut heard = Vec::new();
        while let Ok(Some(message)) = person.channel.heard() {
            if let FromWorker::Started(pid) = message {
                person.program = Some(pid);
            }
            heard.push(message);
        }
        let exited = match person.child.try_wait() {
            Ok(Some(status)) => {
                // Whatever it said before it went is heard first.
                while let Ok(Some(message)) = person.channel.heard() {
                    heard.push(message);
                }
                Some(status)
            }
            _ => None,
        };
        let mut actions = Vec::new();
        for message in heard {
            actions.extend(match message {
                FromWorker::Accepted => self.manager.checked(Checked::Accepted, now),
                FromWorker::Rejected => self.manager.checked(Checked::Rejected, now),
                FromWorker::Failed => self.manager.checked(Checked::Failed, now),
                FromWorker::Started(_) => self.manager.session_started(),
            });
        }
        if let Some(status) = exited {
            if !status.success() {
                eprintln!("[login] a worker failed ({status})");
            }
            self.person = None;
            actions.extend(self.manager.worker_ended(now));
        }
        self.execute(actions);
    }

    /// Waits for something to happen, at most a tenth of a second.
    fn wait(&self) {
        use rustix::event::{poll, PollFd, PollFlags};
        use std::os::fd::AsFd;
        let mut fds = vec![PollFd::from_borrowed_fd(
            self.listener.as_fd(),
            PollFlags::IN,
        )];
        if let Some(connection) = &self.connection {
            fds.push(PollFd::from_borrowed_fd(connection.as_fd(), PollFlags::IN));
        }
        for worker in [&self.greeter, &self.person].into_iter().flatten() {
            fds.push(PollFd::from_borrowed_fd(
                worker.channel.as_fd(),
                PollFlags::IN,
            ));
        }
        let timeout = rustix::time::Timespec {
            tv_sec: 0,
            tv_nsec: 100_000_000,
        };
        let _ = poll(&mut fds, Some(&timeout));
    }

    /// The login program is stopped: its workers end their sessions (their channels closed and
    /// SIGTERM), and are killed if they have not within the grace.
    fn shut_down(&mut self) {
        eprintln!("[login] ending");
        let mut ending = Vec::new();
        for worker in [self.person.take(), self.greeter.take()]
            .into_iter()
            .flatten()
        {
            worker.signal(rustix::process::Signal::TERM);
            // A worker waiting for its next message hears its channel close (a signal does not
            // interrupt that wait): it ends, and the session it holds with it.
            let Worker { child, program, .. } = worker;
            ending.push((child, program));
        }
        let deadline = Instant::now() + Duration::from_secs(8);
        for (child, program) in &mut ending {
            while Instant::now() < deadline {
                if let Ok(Some(_)) = child.try_wait() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            if !matches!(child.try_wait(), Ok(Some(_))) {
                if let Some(group) =
                    program.and_then(|pid| rustix::process::Pid::from_raw(pid as i32))
                {
                    let _ =
                        rustix::process::kill_process_group(group, rustix::process::Signal::KILL);
                }
                let _ = child.kill();
                let _ = child.wait();
            }
        }
        let _ = std::fs::remove_file(&self.config.socket);
    }
}
