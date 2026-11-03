//! The real engine, when one is built (`tools/input-methods/build.sh`) and named by
//! `OXYOS_INPUT_METHODS`: keys typed through the session's frontend become the text fcitx5's
//! engines make of them. Without one, there is nothing to run.

use oxyos_input_methods::{Change, Command, Engine, Outcome, Paths, Service, Sources};
use std::time::{Duration, Instant};

fn engine() -> Option<Engine> {
    Engine::at(std::env::var_os("OXYOS_INPUT_METHODS")?)
}

/// Linux input codes and keysyms of the keys typed: letters and Space.
fn key(c: char) -> (u32, u32) {
    let code = match c {
        'a' => 30,
        'b' => 48,
        'c' => 46,
        'd' => 32,
        'e' => 18,
        'f' => 33,
        'g' => 34,
        'h' => 35,
        'i' => 23,
        'j' => 36,
        'k' => 37,
        'l' => 38,
        'm' => 50,
        'n' => 49,
        'o' => 24,
        'p' => 25,
        'q' => 16,
        'r' => 19,
        's' => 31,
        't' => 20,
        'u' => 22,
        'v' => 47,
        'w' => 17,
        'x' => 45,
        'y' => 21,
        'z' => 44,
        ' ' => 57,
        '\n' => return (28, 0xff0d),
        _ => panic!("no key for {c}"),
    };
    (code, c as u32)
}

struct Typing {
    service: Service,
    serial: u64,
}

impl Typing {
    fn start(engine: Engine, name: &str, current: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("oxyos-im-test-{name}-{}", std::process::id()));
        let paths = Paths {
            runtime: dir.join("run"),
            data: dir.join("data"),
        };
        let sources = Sources {
            layout: "us".into(),
            group: vec!["keyboard-us".into(), current.into()],
            current: current.into(),
        };
        let mut typing = Self {
            service: Service::spawn(engine, paths, sources).unwrap(),
            serial: 0,
        };
        typing.wait(|outcome| matches!(outcome, Outcome::Ready));
        typing.service.send(Command::Activate { capability: 0 });
        typing
    }

    fn wait(&mut self, mut until: impl FnMut(&Outcome) -> bool) -> Vec<Outcome> {
        let start = Instant::now();
        let mut seen = Vec::new();
        loop {
            for outcome in self.service.outcomes() {
                if let Outcome::Stopped(why) = &outcome {
                    panic!("the engine stopped: {why}");
                }
                let done = until(&outcome);
                seen.push(outcome);
                if done {
                    return seen;
                }
            }
            assert!(
                start.elapsed() < Duration::from_secs(30),
                "waited in vain: {seen:?}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Types `text`, press and release each key, and gathers the field's changes.
    fn type_text(&mut self, text: &str) -> Change {
        let mut change = Change::default();
        for c in text.chars() {
            let (code, keysym) = key(c);
            for pressed in [true, false] {
                self.serial += 1;
                let serial = self.serial;
                self.service.send(Command::Key {
                    serial,
                    keysym,
                    code,
                    modifiers: 0,
                    pressed,
                    time_ms: 0,
                });
                for outcome in
                    self.wait(|o| matches!(o, Outcome::Key { serial: s, .. } if *s == serial))
                {
                    match outcome {
                        Outcome::Key { change: c, .. } | Outcome::Change(c) => {
                            if c.delete.is_some() {
                                change.delete = c.delete;
                            }
                            change.commit.push_str(&c.commit);
                            if c.preedit.is_some() {
                                change.preedit = c.preedit;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        change
    }
}

#[test]
fn pinyin_types_nihao_as_its_first_candidate() {
    let Some(engine) = engine() else { return };
    let mut typing = Typing::start(engine, "pinyin", "pinyin");
    let composing = typing.type_text("nihao");
    assert_eq!(composing.commit, "");
    let preedit = composing.preedit.expect("a composition");
    assert!(
        preedit.text().replace(' ', "").contains("nihao"),
        "{preedit:?}"
    );
    let committed = typing.type_text(" ");
    assert_eq!(committed.commit, "你好");
    assert_eq!(committed.preedit.map(|p| p.text()), Some(String::new()));
}

#[test]
fn hangul_composes_syllables() {
    let Some(engine) = engine() else { return };
    let mut typing = Typing::start(engine, "hangul", "hangul");
    // 2-Set: g k s → 한, then a space commits it and types itself.
    let change = typing.type_text("gks");
    assert_eq!(change.preedit.map(|p| p.text()), Some("한".to_owned()));
    let change = typing.type_text(" ");
    assert!(change.commit.starts_with('한'), "{change:?}");
}

#[test]
fn anthy_turns_romaji_into_kana_and_kana_into_kanji() {
    let Some(engine) = engine() else { return };
    let mut typing = Typing::start(engine, "anthy", "anthy");
    let change = typing.type_text("nihongo");
    assert_eq!(
        change.preedit.map(|p| p.text()),
        Some("にほんご".to_owned())
    );
    // Space converts: the clause being converted is highlighted.
    let change = typing.type_text(" ");
    let preedit = change.preedit.expect("the conversion");
    assert_eq!(preedit.text(), "日本語");
    assert!(
        preedit.parts.iter().any(|(_, format)| format.converting()),
        "{preedit:?}"
    );
    // Return commits it.
    let change = typing.type_text("\n");
    assert_eq!(change.commit, "日本語");
}

fn descriptors() -> Vec<String> {
    let mut open: Vec<String> = std::fs::read_dir("/proc/self/fd")
        .unwrap()
        .filter_map(|entry| std::fs::read_link(entry.ok()?.path()).ok())
        .map(|target| target.display().to_string())
        .filter(|target| !target.starts_with("/proc/"))
        .collect();
    open.sort();
    open
}

#[test]
fn the_service_leaves_no_descriptor_open_once_dropped() {
    let Some(engine) = engine() else { return };
    // zbus's reactor (async-io's driver, its epoll, eventfd and timerfd) is the process's, made
    // at its first use and never closed: running before the count, as in a session.
    zbus::block_on(async_io::Timer::after(Duration::from_millis(1)));
    let before = descriptors();
    let typing = Typing::start(engine, "descriptors", "pinyin");
    drop(typing);
    // A socket closed is let go on zbus's own thread a moment later.
    let start = Instant::now();
    while descriptors().len() > before.len() && start.elapsed() < Duration::from_secs(2) {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(descriptors(), before);
}
