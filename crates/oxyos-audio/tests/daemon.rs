//! The session's client against a real PipeWire of the test's own (`common::Daemon`): what it
//! reads of the graph, and what it changes in it, read back by a second connection and by
//! PipeWire's own tools. No WirePlumber runs: the test applies a chosen default as WirePlumber
//! does (`default.audio.sink` from `default.configured.audio.sink`).
//!
//! One test in a binary of its own: the daemon's socket is the process's environment.

mod common;

use common::{null_node, Daemon};
use oxyos_audio::{takeover, Audio, Direction, Event, Kind, PipeWire, State, Transport};
use rustix::event::{poll, PollFd, PollFlags, Timespec};
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

/// Follows PipeWire on a thread until the returned stream is written to.
struct Following {
    order: UnixStream,
    thread: Option<std::thread::JoinHandle<std::io::Result<()>>>,
}

impl Following {
    fn start(audio: &PipeWire) -> Self {
        let (stop, order) = UnixStream::pair().unwrap();
        let audio = audio.clone();
        let thread = std::thread::spawn(move || audio.follow(stop.as_fd()));
        Self {
            order,
            thread: Some(thread),
        }
    }

    fn stop(mut self) -> std::io::Result<()> {
        use std::io::Write;
        self.order.write_all(&[1]).unwrap();
        self.thread.take().unwrap().join().unwrap()
    }

    fn ended(&mut self) -> Option<std::io::Result<()>> {
        let thread = self.thread.take()?;
        if thread.is_finished() {
            Some(thread.join().unwrap())
        } else {
            self.thread = Some(thread);
            None
        }
    }
}

/// Events until a state `wanted` says yes to (or ten seconds), waiting on the client's waker;
/// the state.
fn until(audio: &PipeWire, what: &str, wanted: impl Fn(&State) -> bool) -> State {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        for event in audio.take_events() {
            if let Event::State(state) = event {
                if wanted(&state) {
                    return state;
                }
            }
        }
        if wanted(&audio.state()) {
            return audio.state();
        }
        let left = deadline.saturating_duration_since(Instant::now());
        assert!(
            !left.is_zero(),
            "{what}: the last state {:#?}",
            audio.state()
        );
        let timeout = Timespec {
            tv_sec: 0,
            tv_nsec: left.as_nanos().min(100_000_000) as i64,
        };
        let mut fds = [PollFd::from_borrowed_fd(
            audio.wake_fd().unwrap(),
            PollFlags::IN,
        )];
        let _ = poll(&mut fds, Some(&timeout));
    }
}

/// The state as a new connection reads it whole.
fn fresh(daemon: &Daemon) -> State {
    let _ = daemon;
    let audio = PipeWire::new().unwrap();
    let following = Following::start(&audio);
    let state = until(&audio, "read anew", |state| {
        state.available && state.output.selected().is_some() && state.input.selected().is_some()
    });
    following.stop().unwrap();
    state
}

/// Waits until a node's channels' linear volumes are `wanted`, as PipeWire's own tool reads them.
fn until_channels(daemon: &Daemon, node: &str, wanted: &[f32]) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let props = daemon.cli(&["enum-params", node, "Props"]);
        // The first float array after channelVolumes.
        let channels: Vec<f32> = props
            .split("Props:channelVolumes")
            .nth(1)
            .map(|rest| {
                rest.lines()
                    .skip(2)
                    .take_while(|line| line.trim().starts_with("Float"))
                    .filter_map(|line| line.trim().strip_prefix("Float ")?.parse().ok())
                    .collect()
            })
            .unwrap_or_default();
        if channels.len() == wanted.len()
            && channels
                .iter()
                .zip(wanted)
                .all(|(a, b)| (a - b).abs() < 0.0005)
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{node}'s channels {channels:?}, wanted {wanted:?}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn near(a: Option<f64>, b: f64) -> bool {
    a.is_some_and(|a| (a - b).abs() < 0.002)
}

#[test]
fn the_client_reads_and_changes_a_real_pipewire() {
    let mut daemon = Daemon::start(
        "daemon",
        &[
            null_node("speakers", "Built-in Speakers", "Audio/Sink", ""),
            null_node("hdmi", "Monitor", "Audio/Sink", ""),
            null_node(
                "mic",
                "Built-in Microphone",
                "Audio/Source",
                "device.form-factor = internal",
            ),
        ],
    );
    // WirePlumber's part: each device's ports configured, and its choices at start.
    daemon.configure("speakers", true);
    daemon.configure("hdmi", true);
    daemon.configure("mic", false);
    daemon.set_metadata("default.audio.sink", r#"{"name":"speakers"}"#);
    daemon.set_metadata("default.audio.source", r#"{"name":"mic"}"#);

    let audio = PipeWire::new().unwrap();
    assert!(!audio.state().available, "unavailable until it follows");
    let following = Following::start(&audio);
    // A second connection follows what the first changed, as another program would.
    let reader = PipeWire::new().unwrap();
    let reading = Following::start(&reader);

    let state = until(&audio, "the graph read whole", |state| {
        state.available && state.output.devices.len() == 2 && state.input.devices.len() == 1
    });
    let ids: Vec<&str> = state.output.devices.iter().map(|d| d.id.as_str()).collect();
    // Without a device object, a node is a program's (no hardware), named by its description.
    assert_eq!(ids, ["speakers", "hdmi"]);
    assert_eq!(state.output.devices[0].name, "Built-in Speakers");
    assert_eq!(state.output.devices[0].transport, Transport::Virtual);
    assert!(state.output.devices[0].selected);
    assert!(!state.output.devices[1].selected);
    assert_eq!(state.input.devices[0].kind, Kind::Microphone);
    assert!(state.input.devices[0].selected);
    assert_eq!(state.output.volume, Some(1.0));
    assert_eq!(state.output.balance, Some(0.0));
    assert_eq!(state.input.balance, None, "an input has no balance");

    // A volume: on the cubic scale, as each channel's linear volume.
    audio.set_volume(Direction::Output, 0.5);
    assert_eq!(audio.state().output.volume, Some(0.5), "said at once");
    until_channels(&daemon, "speakers", &[0.125, 0.125]);
    // The balance to the left, then a volume that keeps it.
    audio.set_balance(-0.5);
    until_channels(&daemon, "speakers", &[0.125, 0.015625]);
    audio.set_volume(Direction::Output, 0.8);
    until_channels(&daemon, "speakers", &[0.512, 0.064]);
    // To nothing and back: the balance it had.
    audio.set_volume(Direction::Output, 0.0);
    until_channels(&daemon, "speakers", &[0.0, 0.0]);
    audio.set_volume(Direction::Output, 0.6);
    until_channels(&daemon, "speakers", &[0.216, 0.027]);
    audio.set_muted(Direction::Output, true);
    audio.set_volume(Direction::Input, 0.25);
    until_channels(&daemon, "mic", &[0.015625, 0.015625]);
    // Read anew, as another program would: the volume, the balance and the mute set.
    let read = fresh(&daemon);
    assert!(near(read.output.volume, 0.6), "{read:#?}");
    assert!(near(read.output.balance, -0.5), "{read:#?}");
    assert!(read.output.muted);
    assert!(near(read.input.volume, 0.25));
    audio.set_muted(Direction::Output, false);
    let deadline = Instant::now() + Duration::from_secs(10);
    while fresh(&daemon).output.muted {
        assert!(Instant::now() < deadline, "still muted");
    }

    // Bluetooth headphones connected: listed by their name, and they take over.
    let before = audio.state();
    daemon.cli(&[
        "create-node",
        "adapter",
        &null_node(
            "bluez_output.airpods",
            "AirPods",
            "Audio/Sink",
            "device.api = bluez5 device.form-factor = headphone",
        ),
    ]);
    daemon.configure("bluez_output.airpods", true);
    let after = until(&audio, "the headphones connected", |state| {
        state.output.devices.len() == 3
    });
    let airpods = after
        .output
        .devices
        .iter()
        .find(|device| device.id == "bluez_output.airpods")
        .unwrap();
    assert_eq!(
        (airpods.name.as_str(), airpods.kind, airpods.transport),
        ("AirPods", Kind::Headphones, Transport::Bluetooth)
    );
    assert_eq!(
        takeover(&before, &after),
        [(Direction::Output, "bluez_output.airpods".to_string())]
    );

    // Chosen as wpctl chooses it: WirePlumber's configured default.
    audio.set_default(Direction::Output, "bluez_output.airpods");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !daemon
        .metadata()
        .contains(r#"key:'default.configured.audio.sink' value:'{"name":"bluez_output.airpods"}'"#)
    {
        assert!(Instant::now() < deadline, "{}", daemon.metadata());
        std::thread::sleep(Duration::from_millis(20));
    }
    until(&audio, "said chosen at once", |state| {
        state.output.selected().map(|d| d.id.as_str()) == Some("bluez_output.airpods")
    });
    // WirePlumber applies it; another program sees it chosen.
    daemon.set_metadata("default.audio.sink", r#"{"name":"bluez_output.airpods"}"#);
    until(&reader, "chosen, as another program sees it", |state| {
        state.output.selected().map(|d| d.id.as_str()) == Some("bluez_output.airpods")
    });
    // Nothing that is not an output is chosen.
    audio.set_default(Direction::Output, "mic");
    std::thread::sleep(Duration::from_millis(200));
    assert!(!daemon
        .metadata()
        .contains(r#"key:'default.configured.audio.sink' value:'{"name":"mic"}'"#));

    // Disconnected: gone from the list.
    let id = daemon.node_id("bluez_output.airpods").unwrap().to_string();
    daemon.cli(&["destroy", &id]);
    until(&audio, "the headphones gone", |state| {
        state.output.devices.len() == 2
    });

    // The input metered: a capture stream while asked for, none after.
    audio.meter(true);
    let deadline = Instant::now() + Duration::from_secs(10);
    while daemon.node_id("oxyos-input-level").is_none() {
        assert!(Instant::now() < deadline, "no meter's stream");
        std::thread::sleep(Duration::from_millis(20));
    }
    audio.meter(false);
    let deadline = Instant::now() + Duration::from_secs(10);
    while daemon.node_id("oxyos-input-level").is_some() {
        assert!(Instant::now() < deadline, "the meter's stream kept");
        std::thread::sleep(Duration::from_millis(20));
    }

    // Stopped: unavailable, and it ended well.
    reading.stop().unwrap();
    following.stop().unwrap();
    assert!(!audio.state().available);

    // PipeWire going away ends it with an error, unavailable.
    let audio = PipeWire::new().unwrap();
    let mut following = Following::start(&audio);
    until(&audio, "read again", |state| state.available);
    daemon.stop();
    let deadline = Instant::now() + Duration::from_secs(10);
    let ended = loop {
        if let Some(ended) = following.ended() {
            break ended;
        }
        assert!(Instant::now() < deadline, "still following a daemon gone");
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(ended.is_err());
    assert!(!audio.state().available);
}
