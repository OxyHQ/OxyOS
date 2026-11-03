//! OxyOS's screen reader (docs/adr/0014-screen-reader.mdx): an assistive technology the session
//! starts on the reader's socket (`OXYOS_COMPOSITOR_SOCKET`), granted `accessibility` and
//! `screen_reader`. It holds every surface's accessibility tree, says the element focused, what
//! is typed, deleted and selected in a focused text field and what applications announce, and
//! moves through the elements and activates them with its keys
//! (Ctrl+Alt+Right, Ctrl+Alt+Left, Ctrl+Alt+Space), which the compositor sends it alone.
//!
//! It speaks the session's language (Spanish or English) with that language's voice, from the
//! one `OXYOS_SPEECH` names: eSpeak NG by default, or `record:PATH` for scenarios. The language is
//! the session's locale (`Locale`, docs/adr/0040-language-and-region.mdx), told when the reader
//! connects and whenever it changes, from the next thing it says; until told, its environment's
//! (`LC_ALL`/`LC_MESSAGES`/`LANG`). It says it is on once the events that come with Welcome are
//! in, in the session's language.
//!
//! `oxyos-reader`

use oxyos_client::{Client, Poll};
use oxyos_protocol::{Event, Politeness};
use oxyos_speech::{Language, Speaker};
use rustix::event::{poll, PollFd, PollFlags, Timespec};
use std::os::fd::AsFd;
use std::time::{Duration, Instant};

mod editing;
mod reader;
#[cfg(test)]
mod tests;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// How often a voice is asked whether it finished, while something waits to be said.
const SPEAKING_POLL: Duration = Duration::from_millis(20);

/// How long the reader waits, once connected, for the session to tell its locale before it says
/// it is on: a session tells it with Welcome; a compositor that tells none leaves the reader in
/// its environment's language.
const LOCALE_WAIT: Duration = Duration::from_millis(250);

/// The protocol version whose compositors tell the session's locale (ADR 0033).
const LOCALE_VERSION: u16 = 17;

/// Waits until the compositor has sent something, or `timeout` passes.
fn wait(client: &Client, timeout: Option<Duration>) -> Result<()> {
    let timeout = timeout.map(|timeout| Timespec {
        tv_sec: timeout.as_secs() as _,
        tv_nsec: timeout.subsec_nanos() as _,
    });
    let mut fds = [PollFd::from_borrowed_fd(client.as_fd(), PollFlags::IN)];
    match poll(&mut fds, timeout.as_ref()) {
        Ok(_) | Err(rustix::io::Errno::INTR) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

/// Takes in what the compositor sent, answering the reader's commands; false once it closed the
/// connection.
fn serve(client: &mut Client, reader: &mut reader::Reader) -> Result<bool> {
    loop {
        match client.poll()? {
            Poll::Event(Event::ScreenReaderCommand { command }) => {
                if let Some(request) = reader.command(command) {
                    client.send(&request)?;
                }
            }
            Poll::Event(event) => reader.receive(&event),
            Poll::Empty => return Ok(true),
            Poll::Closed => return Ok(false),
        }
    }
}

fn main() -> Result<()> {
    let socket = std::env::var_os("OXYOS_COMPOSITOR_SOCKET")
        .ok_or("usage: OXYOS_COMPOSITOR_SOCKET=SOCKET oxyos-reader")?;
    let language = Language::from_env();
    let mut speaker = Speaker::new(oxyos_speech::from_env(language)?);
    let mut client = Client::connect(socket.as_ref())?;
    let mut reader = reader::Reader::new(language);
    // The events that come with Welcome first: the session's locale among them (ADR 0040), so
    // that the reader says it is on in the session's language.
    if client.version() >= LOCALE_VERSION {
        let deadline = Instant::now() + LOCALE_WAIT;
        while !reader.told_locale() {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            wait(&client, Some(left))?;
            if !serve(&mut client, &mut reader)? {
                return Ok(());
            }
        }
    }
    let mut failed = false;
    let mut say = |speaker: &mut Speaker<Box<dyn oxyos_speech::Backend>>,
                   text: &str,
                   politeness,
                   language| {
        speaker.set_language(language);
        // A voice that cannot speak is said once, not on every utterance.
        if let Err(error) = speaker.say(text, politeness) {
            if !std::mem::replace(&mut failed, true) {
                eprintln!("[reader] cannot speak: {error}");
            }
        }
    };
    let started = match reader.language() {
        Language::Spanish => "Lector de pantalla activado",
        Language::English => "Screen reader on",
    };
    say(
        &mut speaker,
        started,
        Politeness::Assertive,
        reader.language(),
    );
    loop {
        for utterance in reader.take_said() {
            say(
                &mut speaker,
                &utterance.text,
                utterance.politeness,
                utterance.language,
            );
        }
        if let Err(error) = speaker.pump() {
            eprintln!("[reader] cannot speak: {error}");
        }
        wait(&client, speaker.busy().then_some(SPEAKING_POLL))?;
        if !serve(&mut client, &mut reader)? {
            return Ok(());
        }
    }
}
