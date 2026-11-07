//! The programs every step runs, through one seam: [`System`] runs them on the machine; the tests'
//! host records them and answers for them.
//!
//! Every program is Debian's or systemd's own (systemd-repart, cryptsetup, mount, chroot, apt-get,
//! kernel-install through update-initramfs, efibootmgr through systemd-boot's own maintainer
//! script): nothing here formats, partitions or encrypts by itself.

use std::fmt;
use std::io::{Read, Write};
use std::process::{Command as Process, Stdio};
use zeroize::Zeroizing;

/// A program to run.
#[derive(Clone, PartialEq, Eq)]
pub struct Command {
    pub program: String,
    pub arguments: Vec<String>,
    /// Environment added to the program's.
    pub environment: Vec<(String, String)>,
    /// What it reads on its standard input (a passphrase): overwritten once used, never printed.
    pub input: Option<Zeroizing<Vec<u8>>>,
}

impl Command {
    pub fn new(program: &str) -> Self {
        Self {
            program: program.into(),
            arguments: Vec::new(),
            environment: Vec::new(),
            input: None,
        }
    }

    pub fn arg(mut self, argument: impl Into<String>) -> Self {
        self.arguments.push(argument.into());
        self
    }

    pub fn args<I: IntoIterator<Item = S>, S: Into<String>>(mut self, arguments: I) -> Self {
        self.arguments.extend(arguments.into_iter().map(Into::into));
        self
    }

    pub fn env(mut self, name: &str, value: &str) -> Self {
        self.environment.push((name.into(), value.into()));
        self
    }

    pub fn input(mut self, bytes: &[u8]) -> Self {
        self.input = Some(Zeroizing::new(bytes.to_vec()));
        self
    }

    /// The command line as the log and the tests show it (never its input).
    pub fn line(&self) -> String {
        let mut line = self.program.clone();
        for argument in &self.arguments {
            line.push(' ');
            line.push_str(argument);
        }
        line
    }
}

impl fmt::Debug for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.line())?;
        if self.input.is_some() {
            write!(f, " <input>")?;
        }
        Ok(())
    }
}

/// How a program ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Output {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Output {
    pub fn ok(stdout: &str) -> Self {
        Self {
            status: 0,
            stdout: stdout.into(),
            stderr: String::new(),
        }
    }
}

/// A step that failed: what ran, and what it said.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failed {
    pub what: String,
    pub detail: String,
}

impl fmt::Display for Failed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.detail.is_empty() {
            write!(f, "{}", self.what)
        } else {
            write!(f, "{}: {}", self.what, self.detail)
        }
    }
}

impl std::error::Error for Failed {}

pub fn failed(what: impl Into<String>, detail: impl Into<String>) -> Failed {
    Failed {
        what: what.into(),
        detail: detail.into(),
    }
}

/// Runs programs.
pub trait Host {
    /// Runs `command` to its end, whatever its status.
    fn execute(&mut self, command: &Command) -> Result<Output, Failed>;

    /// Runs `command` and requires it to succeed; returns its standard output.
    fn run(&mut self, command: &Command) -> Result<String, Failed> {
        let output = self.execute(command)?;
        if output.status == 0 {
            Ok(output.stdout)
        } else {
            let said = output.stderr.trim();
            let said = if said.is_empty() {
                output.stdout.trim()
            } else {
                said
            };
            Err(failed(
                format!("{} failed ({})", command.program, output.status),
                said.lines()
                    .rev()
                    .take(6)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect::<Vec<_>>()
                    .join("\n"),
            ))
        }
    }
}

/// The machine's own programs.
#[derive(Default)]
pub struct System;

impl Host for System {
    fn execute(&mut self, command: &Command) -> Result<Output, Failed> {
        let mut process = Process::new(&command.program);
        process
            .args(&command.arguments)
            .envs(
                command
                    .environment
                    .iter()
                    .map(|(k, v)| (k.as_str(), v.as_str())),
            )
            .stdin(if command.input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = process.spawn().map_err(|e| {
            failed(
                format!("{} could not start", command.program),
                e.to_string(),
            )
        })?;
        if let Some(input) = &command.input {
            let mut stdin = child.stdin.take().expect("piped");
            stdin
                .write_all(input)
                .map_err(|e| failed(format!("{}'s input", command.program), e.to_string()))?;
        }
        // Read both pipes to their end without one filling while the other is read.
        let mut stderr = child.stderr.take().expect("piped");
        let errors = std::thread::spawn(move || {
            let mut text = String::new();
            let _ = stderr.read_to_string(&mut text);
            text
        });
        let mut stdout = String::new();
        let _ = child
            .stdout
            .take()
            .expect("piped")
            .read_to_string(&mut stdout);
        let status = child
            .wait()
            .map_err(|e| failed(format!("{} did not end", command.program), e.to_string()))?;
        let stderr = errors.join().unwrap_or_default();
        Ok(Output {
            status: status.code().unwrap_or(-1),
            stdout,
            stderr,
        })
    }
}
