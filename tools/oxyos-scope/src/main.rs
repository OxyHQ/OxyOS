//! `oxyos-scope [--fixture DIR] [--description TEXT] (app ID | background NAME) -- PROGRAM [ARGS...]`:
//! how the session starts an application in a control group of its own
//! (docs/adr/0066-quality-of-service.mdx, decision 3). It asks the person's user manager for a scope
//! with its own pid (`app-oxyos-<ID>-<pid>.scope` in `app.slice`, or `oxyos-<NAME>-<pid>.scope` in
//! `background.slice`), waits for the job that moves it there, then executes PROGRAM with ARGS in
//! the same process, its environment and descriptors as they came: the launcher's child, its pid
//! and its pidfd are the application's, and nothing the program does happens outside its scope.
//! With `--fixture`, the scope is made in that control group instead (the saturation benchmark's,
//! docs/performance.mdx). A scope that cannot be had (no user bus, the manager refusing or not
//! answering in time) is said once on standard error and the program runs where it was started, as
//! before there were scopes: an application always starts.

use oxyos_scopes::{Manager, Unit};
use std::ffi::OsString;
use std::os::unix::process::CommandExt;
use std::process::{Command, ExitCode};

const USAGE: &str = "usage: oxyos-scope [--fixture DIR] [--description TEXT] \
                     (app ID | background NAME) -- PROGRAM [ARGS...]";

fn main() -> ExitCode {
    let mut arguments = std::env::args_os().skip(1);
    let mut manager = Manager::User;
    let mut description = None;
    let mut unit = None;
    loop {
        let Some(argument) = arguments.next() else {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        };
        match argument.to_str() {
            Some("--fixture") => match arguments.next() {
                Some(dir) => manager = Manager::Fixture(dir.into()),
                None => return usage(),
            },
            Some("--description") => match arguments.next().and_then(|d| d.into_string().ok()) {
                Some(text) => description = Some(text),
                None => return usage(),
            },
            Some(kind @ ("app" | "background")) => {
                let Some(name) = arguments.next().and_then(|n| n.into_string().ok()) else {
                    return usage();
                };
                let pid = std::process::id();
                unit = Some(if kind == "app" {
                    Unit::application(&name, pid)
                } else {
                    Unit::background(&name, pid)
                });
                if description.is_none() {
                    description = Some(name);
                }
            }
            Some("--") => break,
            _ => return usage(),
        }
    }
    let (Some(unit), Some(program)) = (unit, arguments.next()) else {
        return usage();
    };
    let rest: Vec<OsString> = arguments.collect();
    if let Err(error) = oxyos_scopes::place(
        &manager,
        &unit,
        description.as_deref().unwrap_or_default(),
        std::process::id(),
    ) {
        eprintln!(
            "[scope] {}: {error}; it runs where it was started",
            unit.name
        );
    }
    let error = Command::new(&program).args(rest).exec();
    eprintln!("[scope] {}: {error}", program.to_string_lossy());
    ExitCode::from(127)
}

fn usage() -> ExitCode {
    eprintln!("{USAGE}");
    ExitCode::from(2)
}
