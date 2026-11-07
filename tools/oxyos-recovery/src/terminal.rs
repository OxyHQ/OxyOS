//! Reading a person's answers on the console: lines, and secrets typed without echo.

use rustix::termios::{self, LocalModes, OptionalActions};
use std::io::{self, BufRead, Write};
use zeroize::Zeroizing;

/// A line typed after `prompt`, without its end of line. End of input is an error: nobody is
/// there to answer.
pub fn read_line(prompt: &str) -> io::Result<String> {
    print!("{prompt}");
    io::stdout().flush()?;
    let mut line = String::new();
    if io::stdin().lock().read_line(&mut line)? == 0 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "the console closed",
        ));
    }
    Ok(line.trim_end_matches(['\n', '\r']).to_string())
}

/// A secret typed after `prompt`, not shown as it is typed when the console is a terminal, and
/// overwritten in memory once dropped.
pub fn read_secret(prompt: &str) -> io::Result<Zeroizing<String>> {
    let stdin = io::stdin();
    let saved = termios::tcgetattr(&stdin).ok();
    if let Some(saved) = &saved {
        let mut quiet = saved.clone();
        quiet.local_modes.remove(LocalModes::ECHO);
        quiet.local_modes.insert(LocalModes::ECHONL);
        let _ = termios::tcsetattr(&stdin, OptionalActions::Now, &quiet);
    }
    let read = read_line(prompt).map(Zeroizing::new);
    if let Some(saved) = &saved {
        let _ = termios::tcsetattr(&stdin, OptionalActions::Now, saved);
    }
    read
}
