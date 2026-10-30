//! `oxyos-login`, OxyOS's own login program on the seat ([ADR 0020](../../../docs/adr/0020-oxy-account-login.mdx),
//! decision 4, step 2). A system service, root, started at boot after the boot logo
//! (`oxyos-login.service`, docs/adr/0017-boot-logo.mdx). Its parts, as a login manager's:
//!
//! - **The login screen** runs as the login program's own user, `oxyos-login`, in a logind session
//!   of class `greeter` on seat0's first terminal that a worker opens for it (PAM service
//!   `oxyos-login-greeter`): the session program in its login screen mode (`oxyos-session
//!   --greeter`), the compositor owning the card through logind, with the Bloom host's
//!   `OxyOSLogin` as its one client. It lists the enrolled accounts (the accounts' service's
//!   `list`) and takes a password, and decides nothing.
//! - **The password** goes from the login screen's lock surface through the compositor
//!   (`AuthenticateAccount`, `AuthenticateSecret`, `Authenticate`) to this program
//!   (`oxyos_auth::greeter`), never to JavaScript with authority. For an account the accounts'
//!   service lists, a worker checks it with PAM (service `oxyos-login`), throttled as the lock is.
//! - **Signing in.** Accepted: the login screen shows the splash and ends, its session closed and
//!   the card let go; then the worker opens the person's session (`pam_systemd`: a logind session
//!   of class `user` on the same seat and terminal) and starts `oxyos-session` as that user, with
//!   a clean environment. Signing out ends it, and the login screen starts again.
//!
//! The decisions are `manager.rs`'s, a state machine tested without a seat; `run.rs` carries them
//! out; `worker.rs` is a worker's process (`oxyos-login --worker`); `channel.rs` what they say.
//!
//! `oxyos-login [--session PROGRAM] [--host BLOOM_HOST] [--bundle BUNDLE] [--vt N]
//! [--greeter-user USER] [--socket PATH] [--accountd PATH]`
//!
//! `oxyos-login --dev --pam-confdir DIR --record FILE [--socket PATH] [--accountd PATH]`: the
//! development path, as the developer's own user. No login screen is started: the development
//! compositor (`oxyos-offscreen --launch --login PATH`) connects as one. Passwords are checked by
//! PAM with the configuration in DIR (`pam_start_confdir`), the person's PAM session is opened
//! and closed there, and what would start is written to FILE, never started.

mod channel;
mod manager;
mod run;
mod worker;

use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "usage: oxyos-login [--session PROGRAM] [--host BLOOM_HOST] \
                     [--bundle BUNDLE] [--vt N] [--greeter-user USER] [--socket PATH] \
                     [--accountd PATH] | --dev --pam-confdir DIR --record FILE [--socket PATH] \
                     [--accountd PATH]";

fn options(mut args: impl Iterator<Item = String>) -> Result<run::Config, String> {
    let mut config = run::Config {
        dev: false,
        confdir: None,
        record: None,
        greeter_user: "oxyos-login".into(),
        socket: oxyos_auth::greeter::SOCKET.into(),
        accountd: oxyos_account::ipc::SOCKET.into(),
        seat: "seat0".into(),
        vt: 1,
        session: "/usr/bin/oxyos-session".into(),
        host: "/usr/libexec/oxyos/oxyos-bloom-host".into(),
        bundle: "/usr/share/oxyos/bloom/index.js".into(),
    };
    while let Some(arg) = args.next() {
        if arg == "--dev" {
            config.dev = true;
            continue;
        }
        let value = args
            .next()
            .ok_or_else(|| format!("{arg} needs a value\n{USAGE}"))?;
        let path = || PathBuf::from(&value);
        match arg.as_str() {
            "--session" => config.session = path(),
            "--host" => config.host = path(),
            "--bundle" => config.bundle = path(),
            "--vt" => config.vt = value.parse().map_err(|_| USAGE)?,
            "--greeter-user" => config.greeter_user = value,
            "--socket" => config.socket = path(),
            "--accountd" => config.accountd = path(),
            "--pam-confdir" => config.confdir = Some(path()),
            "--record" => config.record = Some(path()),
            _ => return Err(format!("unknown argument {arg}\n{USAGE}")),
        }
    }
    // The development path never starts a session, and a real one never reads PAM from
    // anywhere but /etc/pam.d.
    if config.dev != (config.confdir.is_some() && config.record.is_some())
        || (!config.dev && (config.confdir.is_some() || config.record.is_some()))
    {
        return Err(format!(
            "--dev goes with --pam-confdir and --record\n{USAGE}"
        ));
    }
    Ok(config)
}

fn main() -> ExitCode {
    // Before any thread or child: nothing it starts inherits the service manager's socket.
    let notify = oxyos_seat::notify::Notify::take();
    let mut args = std::env::args_os().skip(1).peekable();
    let result = if args.peek().is_some_and(|arg| arg == "--worker") {
        args.next();
        worker::Options::parse(args).and_then(worker::run)
    } else {
        match args
            .map(|arg| arg.into_string())
            .collect::<Result<Vec<_>, _>>()
        {
            Ok(args) => options(args.into_iter()).and_then(|config| run::run(config, notify)),
            Err(_) => Err(USAGE.into()),
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("oxyos-login: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Result<run::Config, String> {
        options(text.split_whitespace().map(String::from))
    }

    #[test]
    fn the_development_path_and_a_real_login_are_told_apart() {
        let real = parse("--vt 2 --session /bin/s").unwrap();
        assert!(!real.dev);
        assert_eq!(real.vt, 2);
        assert_eq!(real.greeter_user, "oxyos-login");
        let dev = parse("--dev --pam-confdir /tmp/p --record /tmp/r").unwrap();
        assert!(dev.dev);
        // A real login never reads PAM from elsewhere nor records instead of starting.
        assert!(parse("--pam-confdir /tmp/p").is_err());
        assert!(parse("--record /tmp/r").is_err());
        assert!(parse("--dev --record /tmp/r").is_err());
        assert!(parse("--bogus x").is_err());
    }
}
