//! OxyOS's screen reader (docs/adr/0014-screen-reader.mdx): an assistive technology the session
//! starts on the reader's socket (`OXYOS_COMPOSITOR_SOCKET`), granted `accessibility` and
//! `screen_reader`. It holds every surface's accessibility tree, says the element focused and
//! what applications announce, and moves through the elements and activates them with its keys
//! (Ctrl+Alt+Right, Ctrl+Alt+Left, Ctrl+Alt+Space), which the compositor sends it alone.
//!
//! It speaks the session's language (Spanish or English, `LC_ALL`/`LC_MESSAGES`/`LANG`) with the
//! voice `OXYOS_SPEECH` names: eSpeak NG by default, or `record:PATH` for scenarios.
//!
//! `oxyos-reader`

use oxyos_client::{Client, Poll};
use oxyos_protocol::{Event, Politeness};
use oxyos_speech::{Language, Speaker};
use rustix::event::{poll, PollFd, PollFlags, Timespec};
use std::os::fd::AsFd;

mod reader;
#[cfg(test)]
mod tests;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// How often a voice is asked whether it finished, while something waits to be said.
const SPEAKING_POLL: Timespec = Timespec {
    tv_sec: 0,
    tv_nsec: 20_000_000,
};

fn main() -> Result<()> {
    let socket = std::env::var_os("OXYOS_COMPOSITOR_SOCKET")
        .ok_or("usage: OXYOS_COMPOSITOR_SOCKET=SOCKET oxyos-reader")?;
    let language = Language::from_env();
    let mut speaker = Speaker::new(oxyos_speech::from_env(language)?);
    let mut client = Client::connect(socket.as_ref())?;
    let mut reader = reader::Reader::new(language);
    let mut failed = false;
    let mut say =
        |speaker: &mut Speaker<Box<dyn oxyos_speech::Backend>>, text: &str, politeness| {
            // A voice that cannot speak is said once, not on every utterance.
            if let Err(error) = speaker.say(text, politeness) {
                if !std::mem::replace(&mut failed, true) {
                    eprintln!("[reader] cannot speak: {error}");
                }
            }
        };
    let started = match language {
        Language::Spanish => "Lector de pantalla activado",
        Language::English => "Screen reader on",
    };
    say(&mut speaker, started, Politeness::Assertive);
    loop {
        let waiting = speaker.busy();
        let mut fds = [PollFd::from_borrowed_fd(client.as_fd(), PollFlags::IN)];
        match poll(&mut fds, waiting.then_some(&SPEAKING_POLL)) {
            Ok(_) | Err(rustix::io::Errno::INTR) => {}
            Err(error) => return Err(error.into()),
        }
        loop {
            match client.poll()? {
                Poll::Event(Event::ScreenReaderCommand { command }) => {
                    if let Some(request) = reader.command(command) {
                        client.send(&request)?;
                    }
                }
                Poll::Event(event) => reader.receive(&event),
                Poll::Empty => break,
                Poll::Closed => return Ok(()),
            }
        }
        for utterance in reader.take_said() {
            say(&mut speaker, &utterance.text, utterance.politeness);
        }
        if let Err(error) = speaker.pump() {
            eprintln!("[reader] cannot speak: {error}");
        }
    }
}
