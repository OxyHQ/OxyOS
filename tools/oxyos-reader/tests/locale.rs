//! The reader against a compositor played here, through a real socket: told the session's locale
//! with Welcome, it says it is on in that language and voice, whatever its environment's
//! (docs/adr/0040-language-and-region.mdx); a compositor of version 14 tells none, and it speaks
//! its environment's.

use oxyos_ipc::{Connection, Listener};
use oxyos_protocol::{Event, PreeditText, ServerConnection};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

fn scratch(name: &str) -> PathBuf {
    let directory =
        std::env::temp_dir().join(format!("oxyos-reader-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    directory
}

/// The reader started with `LC_ALL=environment`, connected to a compositor played here offering
/// at most `version`, which is sent `told` with Welcome; the record's lines once it said one.
fn first_said(name: &str, environment: &str, version: u16, told: &[Event]) -> String {
    let directory = scratch(name);
    let (socket, record) = (directory.join("socket"), directory.join("speech.jsonl"));
    let listener = Listener::bind(&socket).unwrap();
    let mut reader = std::process::Command::new(env!("CARGO_BIN_EXE_oxyos-reader"))
        .env("OXYOS_COMPOSITOR_SOCKET", &socket)
        .env("OXYOS_SPEECH", format!("record:{}", record.display()))
        .env("LC_ALL", environment)
        .spawn()
        .unwrap();
    let server: Connection = listener.accept().unwrap();
    let mut protocol = ServerConnection::new();
    let (hello, _) = server.recv_request().unwrap();
    let welcome = protocol.receive(&hello).unwrap().unwrap();
    let welcome = match welcome {
        Event::Welcome { version: offered } => Event::Welcome {
            version: offered.min(version),
        },
        other => other,
    };
    server.send_event(&welcome, None).unwrap();
    for event in told {
        server.send_event(event, None).unwrap();
    }
    // One turn of the compositor's, ended as a compositor of version 42 ends it.
    server.send_event(&Event::TurnEnd {}, None).unwrap();
    let said = wait_for_line(&record);
    let _ = reader.kill();
    let _ = reader.wait();
    std::fs::remove_dir_all(&directory).unwrap();
    said
}

fn wait_for_line(record: &Path) -> String {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let text = std::fs::read_to_string(record).unwrap_or_default();
        if let Some(line) = text.lines().next().filter(|_| text.ends_with('\n')) {
            return line.to_string();
        }
        assert!(Instant::now() < deadline, "the reader said nothing");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn it_says_it_is_on_in_the_sessions_language() {
    let locale = Event::Locale {
        index: 0,
        last: true,
        text: PreeditText::new("es-ES").unwrap(),
    };
    let said = first_said("spanish", "en_US.UTF-8", oxyos_protocol::VERSION, &[locale]);
    assert!(
        said.ends_with(r#""text":"Lector de pantalla activado","interrupting":true,"voice":"es"}"#),
        "{said}"
    );
}

#[test]
fn told_no_locale_it_speaks_its_environments() {
    let said = first_said("environment", "es_ES.UTF-8", 14, &[]);
    assert!(
        said.ends_with(r#""text":"Lector de pantalla activado","interrupting":true,"voice":"es"}"#),
        "{said}"
    );
    let said = first_said("english", "C", 14, &[]);
    assert!(
        said.ends_with(r#""text":"Screen reader on","interrupting":true,"voice":"en"}"#),
        "{said}"
    );
}
