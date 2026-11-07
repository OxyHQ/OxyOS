//! What the Bluetooth tests share: a bus of the test's own, the played BlueZ on it
//! (`oxyos-fake-bluez`, a machine described in JSON), and the session's connection to it
//! (`oxyos_bluetooth::run`) on a thread, its events received in order.

#![allow(dead_code)]

use std::io::{BufRead, BufReader};
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub struct Played {
    pub address: String,
    pub directory: std::path::PathBuf,
    children: Vec<std::process::Child>,
}

impl Played {
    /// A bus and the played BlueZ, the machine `machine` describes.
    pub fn start(name: &str, machine: &str) -> Played {
        let directory =
            std::env::temp_dir().join(format!("oxyos-bluetooth-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let mut bus = Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address=1"])
            .arg(format!(
                "--address=unix:path={}",
                directory.join("bus").display()
            ))
            .stdout(Stdio::piped())
            .spawn()
            .expect("dbus-daemon (the dbus-daemon package)");
        let mut address = String::new();
        BufReader::new(bus.stdout.take().unwrap())
            .read_line(&mut address)
            .unwrap();
        let address = address.trim().to_string();
        std::fs::write(directory.join("machine.json"), machine).unwrap();
        let fake = Command::new(env!("CARGO_BIN_EXE_oxyos-fake-bluez"))
            .args(["--address", &address, "--machine"])
            .arg(directory.join("machine.json"))
            .arg("--state")
            .arg(directory.join("state.json"))
            .spawn()
            .unwrap();
        let played = Played {
            address,
            directory,
            children: vec![bus, fake],
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        while played.state().is_none() {
            assert!(Instant::now() < deadline, "the played BlueZ did not start");
            std::thread::sleep(Duration::from_millis(20));
        }
        played
    }

    /// What the played BlueZ holds, as it says it.
    pub fn state(&self) -> Option<serde_json::Value> {
        let bus = zbus::blocking::connection::Builder::address(self.address.as_str())
            .ok()?
            .build()
            .ok()?;
        let reply = bus
            .call_method(
                Some("org.bluez"),
                "/so/oxy/OxyOS/FakeBlueZ",
                Some("so.oxy.OxyOS.FakeBlueZ"),
                "State",
                &(),
            )
            .ok()?;
        let text: String = reply.body().deserialize().ok()?;
        serde_json::from_str(&text).ok()
    }

    /// The played machine changes by itself. Through `busctl`, so this process makes no connection
    /// of its own meanwhile (whose runtime's threads would be counted by the idle test).
    pub fn act(&self, what: &str) {
        let status = Command::new("busctl")
            .arg(format!("--address={}", self.address))
            .args([
                "call",
                "org.bluez",
                "/so/oxy/OxyOS/FakeBlueZ",
                "so.oxy.OxyOS.FakeBlueZ",
                "Act",
                "s",
                what,
            ])
            .status()
            .expect("busctl (systemd)");
        assert!(status.success(), "the played BlueZ refused {what}");
    }

    /// Waits until what the played BlueZ holds satisfies `ok`.
    pub fn until(&self, what: &str, ok: impl Fn(&serde_json::Value) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if self.state().is_some_and(|state| ok(&state)) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "the played BlueZ never had {what}: {:?}",
                self.state()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Played {
    fn drop(&mut self) {
        for child in &mut self.children {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

/// The session's connection to the played BlueZ, on a thread.
pub struct Session {
    pub commands: async_channel::Sender<oxyos_bluetooth::Command>,
    pub events: mpsc::Receiver<oxyos_bluetooth::Event>,
    order: UnixStream,
    thread: Option<std::thread::JoinHandle<std::io::Result<()>>>,
    pub calls: u64,
    pub state: oxyos_bluetooth::State,
}

impl Session {
    pub fn start(played: &Played) -> Session {
        let (stop, order) = UnixStream::pair().unwrap();
        let (commands, receiving) = async_channel::unbounded();
        let (told, events) = mpsc::channel();
        let address = played.address.clone();
        let thread = std::thread::Builder::new()
            .name("bluetooth".into())
            .spawn(move || {
                oxyos_bluetooth::run(Some(&address), stop.as_fd(), &receiving, |event| {
                    let _ = told.send(event);
                })
            })
            .unwrap();
        let mut session = Session {
            commands,
            events,
            order,
            thread: Some(thread),
            calls: 0,
            state: oxyos_bluetooth::State::default(),
        };
        session.until_state("BlueZ read", |state| state.available);
        session
    }

    /// The next event, the state kept as it is told.
    pub fn next(&mut self) -> oxyos_bluetooth::Event {
        let event = self
            .events
            .recv_timeout(Duration::from_secs(10))
            .expect("an event");
        if let oxyos_bluetooth::Event::State(state) = &event {
            self.state = state.clone();
        }
        event
    }

    pub fn until_state(&mut self, what: &str, ok: impl Fn(&oxyos_bluetooth::State) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !ok(&self.state) {
            assert!(
                Instant::now() < deadline,
                "never told {what}: {:?}",
                self.state
            );
            match self.events.recv_timeout(Duration::from_millis(200)) {
                Ok(oxyos_bluetooth::Event::State(state)) => self.state = state,
                Ok(other) => panic!("waiting for {what}, told {other:?}"),
                Err(_) => {}
            }
        }
    }

    pub fn send(&self, command: oxyos_bluetooth::Command) {
        self.commands.send_blocking(command).unwrap();
    }

    pub fn call(&mut self) -> u64 {
        self.calls += 1;
        self.calls
    }

    /// The outcome of `call`, states told and questions ended meanwhile kept.
    pub fn done(&mut self, call: u64) -> Result<oxyos_bluetooth::Done, oxyos_bluetooth::Failure> {
        loop {
            match self.next() {
                oxyos_bluetooth::Event::Done {
                    call: done,
                    outcome,
                } if done == call => return outcome,
                oxyos_bluetooth::Event::State(_) | oxyos_bluetooth::Event::QuestionEnded(_) => {}
                other => panic!("waiting for call {call}, told {other:?}"),
            }
        }
    }

    /// The next question of the agent's, states told meanwhile kept.
    pub fn question(&mut self) -> oxyos_bluetooth::Question {
        loop {
            match self.next() {
                oxyos_bluetooth::Event::Question(question) => return question,
                oxyos_bluetooth::Event::State(_) => {}
                other => panic!("waiting for a question, told {other:?}"),
            }
        }
    }

    /// Stops it, waiting for the thread; how long that took.
    pub fn stop(mut self) -> Duration {
        use std::io::Write;
        let stopping = Instant::now();
        self.order.write_all(&[1]).unwrap();
        self.thread.take().unwrap().join().unwrap().unwrap();
        stopping.elapsed()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        use std::io::Write;
        if let Some(thread) = self.thread.take() {
            let _ = self.order.write_all(&[1]);
            let _ = thread.join();
        }
    }
}

/// The reference machine: an adapter on; paired AirPods Pro (connected, its battery 80 %), a
/// keyboard and a mouse; nearby, a phone that compares a code, an old keyboard asking a PIN typed
/// on it, a speaker that just works, a headset with a legacy PIN and one that will not connect.
pub const MACHINE: &str = r#"{
  "adapter": {"name": "ThinkPad", "address": "00:1A:7D:DA:71:13", "powered": true},
  "devices": [
    {"address": "AC:90:85:11:22:33", "name": "AirPods Pro", "icon": "audio-headphones", "paired": true,
     "connected": true, "battery": 80, "uuids": ["0000110b-0000-1000-8000-00805f9b34fb"]},
    {"address": "F0:B3:EC:44:55:66", "name": "Magic Keyboard", "icon": "input-keyboard", "paired": true,
     "uuids": ["00001124-0000-1000-8000-00805f9b34fb"]},
    {"address": "D4:5A:66:77:88:99", "name": "MX Master 3S", "icon": "input-mouse", "paired": true,
     "connected": true, "battery": 55},
    {"address": "3C:28:6D:01:02:03", "name": "Pixel 8", "icon": "phone", "nearby": true,
     "pairing": "confirm", "passkey": 482913, "connect": "none"},
    {"address": "00:1F:20:0A:0B:0C", "name": "Old Keyboard", "icon": "input-keyboard", "nearby": true,
     "pairing": "show-passkey", "passkey": 7302},
    {"address": "B8:F6:53:0D:0E:0F", "name": "JBL Flip 6", "icon": "audio-card", "nearby": true},
    {"address": "00:0D:44:AA:BB:CC", "name": "Car Kit", "icon": "audio-headset", "nearby": true,
     "pairing": "pin", "pin": "1234"},
    {"address": "70:BF:92:DD:EE:FF", "name": "Far Speaker", "icon": "audio-card", "nearby": true,
     "connect": "fail"},
    {"address": "5A:11:22:33:44:55", "nearby": true}
  ]
}"#;
