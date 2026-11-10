use super::graph::{
    Availability, DeviceObject, Graph, Node, Props, Route, Stream, Target, Volumes,
};
use super::*;
use pipewire::spa;

fn props(pairs: &[(&str, &str)]) -> Props {
    pairs
        .iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

fn route(device: i32, direction: Direction, name: &str, description: &str, port: &str) -> Route {
    Route {
        index: device * 10,
        device,
        direction: Some(direction),
        name: name.into(),
        description: description.into(),
        available: Availability::Yes,
        info: props(&[("port.type", port)]),
    }
}

fn stereo(volume: f32) -> Option<Volumes> {
    Some(Volumes {
        channels: vec![volume, volume],
        positions: vec![
            spa::sys::SPA_AUDIO_CHANNEL_FL,
            spa::sys::SPA_AUDIO_CHANNEL_FR,
        ],
        mute: false,
    })
}

/// The reference ThinkPad's graph (SOF, UCM): one card whose "Speaker + Headphones" sink plays
/// through the speakers, three HDMI sinks of which one has a display, a digital microphone and a
/// headset microphone with nothing in the jack.
fn thinkpad() -> Graph {
    let card = 40;
    let mut graph = Graph::default();
    let mut routes = vec![
        route(0, Direction::Output, "[Out] Speaker", "Speaker", "speaker"),
        route(
            1,
            Direction::Output,
            "[Out] HDMI1",
            "HDMI / DisplayPort 1 Output",
            "hdmi",
        ),
        route(
            2,
            Direction::Output,
            "[Out] HDMI2",
            "HDMI / DisplayPort 2 Output",
            "hdmi",
        ),
        route(
            4,
            Direction::Input,
            "[In] Mic1",
            "Digital Microphone",
            "mic",
        ),
        route(
            5,
            Direction::Input,
            "[In] Headset",
            "Headset Microphone",
            "headset",
        ),
    ];
    routes[1]
        .info
        .insert("device.product.name".into(), "DELL U2720Q".into());
    routes[2].available = Availability::No;
    routes[4].available = Availability::No;
    graph.devices.insert(
        card,
        DeviceObject {
            props: props(&[
                ("device.api", "alsa"),
                ("device.bus", "pci"),
                ("device.form-factor", "internal"),
                (
                    "device.description",
                    "Alder Lake PCH-P High Definition Audio Controller",
                ),
            ]),
            routes,
        },
    );
    let node = |name: &str, class: &str, sub: &str, description: &str| Node {
        props: props(&[
            ("node.name", name),
            ("media.class", class),
            ("device.id", "40"),
            ("card.profile.device", sub),
            ("node.description", description),
        ]),
        volumes: stereo(0.125),
    };
    graph.nodes.insert(
        50,
        node(
            "alsa_output.speaker",
            "Audio/Sink",
            "0",
            "Speaker + Headphones",
        ),
    );
    graph.nodes.insert(
        51,
        node(
            "alsa_output.hdmi1",
            "Audio/Sink",
            "1",
            "HDMI / DisplayPort 1 Output",
        ),
    );
    graph.nodes.insert(
        52,
        node(
            "alsa_output.hdmi2",
            "Audio/Sink",
            "2",
            "HDMI / DisplayPort 2 Output",
        ),
    );
    graph.nodes.insert(
        53,
        node("alsa_input.dmic", "Audio/Source", "4", "Digital Microphone"),
    );
    graph.nodes.insert(
        54,
        node(
            "alsa_input.headset",
            "Audio/Source",
            "5",
            "Headphones Stereo Microphone",
        ),
    );
    // A stream is not a device.
    graph.nodes.insert(
        60,
        Node {
            props: props(&[
                ("node.name", "firefox"),
                ("media.class", "Stream/Output/Audio"),
            ]),
            volumes: stereo(1.0),
        },
    );
    graph
        .defaults
        .insert(Direction::Output, "alsa_output.speaker".into());
    graph
        .defaults
        .insert(Direction::Input, "alsa_input.dmic".into());
    graph
}

fn ids(side: &Side) -> Vec<(&str, &str, Kind, Transport, bool)> {
    side.devices
        .iter()
        .map(|d| {
            (
                d.id.as_str(),
                d.name.as_str(),
                d.kind,
                d.transport,
                d.selected,
            )
        })
        .collect()
}

#[test]
fn a_thinkpads_outputs_and_inputs_are_what_is_plugged_in() {
    let state = thinkpad().state(1);
    assert!(state.available);
    // The display's port with no display and the jack's microphone with nothing in it are not
    // listed; the display is named by its EDID.
    assert_eq!(
        ids(&state.output),
        [
            (
                "alsa_output.speaker",
                "Speaker",
                Kind::Speakers,
                Transport::BuiltIn,
                true
            ),
            (
                "alsa_output.hdmi1",
                "DELL U2720Q",
                Kind::Hdmi,
                Transport::Hdmi,
                false
            ),
        ]
    );
    assert_eq!(
        ids(&state.input),
        [(
            "alsa_input.dmic",
            "Digital Microphone",
            Kind::Microphone,
            Transport::BuiltIn,
            true
        )]
    );
    assert_eq!(state.output.volume, Some(0.5));
    assert_eq!(state.output.balance, Some(0.0));
    assert_eq!(state.input.volume, Some(0.5));
    assert_eq!(state.input.balance, None);
}

#[test]
fn headphones_in_the_jack_are_the_same_output_through_its_headphones() {
    let mut graph = thinkpad();
    let before = graph.state(1);
    let card = graph.devices.get_mut(&40).unwrap();
    card.routes[0] = route(
        0,
        Direction::Output,
        "[Out] Headphones",
        "Headphones",
        "headphones",
    );
    card.routes[4].available = Availability::Yes;
    let after = graph.state(1);
    let speaker = &after.output.devices[0];
    assert_eq!(
        (speaker.id.as_str(), speaker.kind, speaker.selected),
        ("alsa_output.speaker", Kind::Headphones, true)
    );
    let headset = after
        .input
        .devices
        .iter()
        .find(|d| d.id == "alsa_input.headset")
        .unwrap();
    assert_eq!(headset.kind, Kind::Headset);
    // The output already plays through them; the headset's microphone takes over.
    assert_eq!(
        takeover(&before, &after),
        [(Direction::Input, "alsa_input.headset".to_string())]
    );
    // With a display's sound chosen, the headphones take it over.
    graph
        .defaults
        .insert(Direction::Output, "alsa_output.hdmi1".into());
    let mut before = before;
    for device in &mut before.output.devices {
        device.selected = device.id == "alsa_output.hdmi1";
    }
    let after = graph.state(1);
    assert!(
        takeover(&before, &after).contains(&(Direction::Output, "alsa_output.speaker".to_string()))
    );
}

fn bluetooth(graph: &mut Graph, form: &str) {
    graph.devices.insert(
        70,
        DeviceObject {
            props: props(&[
                ("device.api", "bluez5"),
                ("device.bus", "bluetooth"),
                ("device.form-factor", form),
                ("device.description", "AirPods Pro"),
            ]),
            routes: vec![route(
                0,
                Direction::Output,
                "headset-output",
                "Headset",
                "headset",
            )],
        },
    );
    graph.nodes.insert(
        71,
        Node {
            props: props(&[
                ("node.name", "bluez_output.AA_BB.1"),
                ("media.class", "Audio/Sink"),
                ("device.id", "70"),
                ("card.profile.device", "0"),
                ("node.description", "AirPods Pro"),
            ]),
            volumes: stereo(1.0),
        },
    );
}

#[test]
fn bluetooth_and_usb_take_over_a_display_does_not_and_a_restart_takes_nothing() {
    let mut graph = thinkpad();
    let before = graph.state(1);
    bluetooth(&mut graph, "headphone");
    let after = graph.state(1);
    let airpods = after
        .output
        .devices
        .iter()
        .find(|d| d.id == "bluez_output.AA_BB.1")
        .unwrap();
    assert_eq!(
        (airpods.name.as_str(), airpods.kind, airpods.transport),
        ("AirPods Pro", Kind::Headset, Transport::Bluetooth)
    );
    // Listed after the built-in and the display's.
    assert_eq!(
        after.output.devices.last().unwrap().id,
        "bluez_output.AA_BB.1"
    );
    assert_eq!(
        takeover(&before, &after),
        [(Direction::Output, "bluez_output.AA_BB.1".to_string())]
    );
    // The same read on a new connection (PipeWire restarted, the session starting): nothing.
    assert!(takeover(&before, &graph.state(2)).is_empty());
    assert!(takeover(&State::unavailable(), &after).is_empty());

    // A display plugged in: listed, chosen only by the person.
    let mut graph = thinkpad();
    let before = graph.state(1);
    graph.devices.get_mut(&40).unwrap().routes[2].available = Availability::Yes;
    let after = graph.state(1);
    assert_eq!(after.output.devices.len(), 3);
    assert!(takeover(&before, &after).is_empty());

    // A USB headset: its output and its microphone.
    let mut graph = thinkpad();
    let before = graph.state(1);
    graph.devices.insert(
        80,
        DeviceObject {
            props: props(&[
                ("device.api", "alsa"),
                ("device.bus", "usb"),
                ("device.form-factor", "headset"),
                ("device.description", "Jabra EVOLVE 65"),
            ]),
            routes: vec![],
        },
    );
    for (id, name, class) in [
        (81, "alsa_output.usb-jabra", "Audio/Sink"),
        (82, "alsa_input.usb-jabra", "Audio/Source"),
    ] {
        graph.nodes.insert(
            id,
            Node {
                props: props(&[
                    ("node.name", name),
                    ("media.class", class),
                    ("device.id", "80"),
                    ("node.description", "Jabra EVOLVE 65 Analog Stereo"),
                ]),
                volumes: stereo(1.0),
            },
        );
    }
    let after = graph.state(1);
    let jabra = after
        .output
        .devices
        .iter()
        .find(|d| d.id == "alsa_output.usb-jabra")
        .unwrap();
    assert_eq!(
        (jabra.name.as_str(), jabra.kind, jabra.transport),
        ("Jabra EVOLVE 65", Kind::Headset, Transport::Usb)
    );
    assert_eq!(
        takeover(&before, &after),
        [
            (Direction::Output, "alsa_output.usb-jabra".to_string()),
            (Direction::Input, "alsa_input.usb-jabra".to_string())
        ]
    );
}

#[test]
fn a_bluetooth_microphone_never_takes_over() {
    let mut graph = thinkpad();
    let before = graph.state(1);
    graph.devices.insert(
        70,
        DeviceObject {
            props: props(&[("device.api", "bluez5"), ("device.form-factor", "headset")]),
            routes: vec![],
        },
    );
    graph.nodes.insert(
        72,
        Node {
            props: props(&[
                ("node.name", "bluez_input.AA_BB.0"),
                ("media.class", "Audio/Source"),
                ("device.id", "70"),
            ]),
            volumes: stereo(1.0),
        },
    );
    let after = graph.state(1);
    assert_eq!(after.input.devices.len(), 2);
    assert!(takeover(&before, &after).is_empty());
}

#[test]
fn a_change_goes_to_the_devices_route_or_a_nodes_own_props() {
    let graph = thinkpad();
    assert_eq!(
        graph.volume_target(Direction::Output),
        Some(Target::Route {
            device: 40,
            index: 0,
            route_device: 0
        })
    );
    assert_eq!(
        graph.volume_target(Direction::Input),
        Some(Target::Route {
            device: 40,
            index: 40,
            route_device: 4
        })
    );
    // A program's sink has no device: its own Props.
    let mut graph = Graph::default();
    graph.nodes.insert(
        9,
        Node {
            props: props(&[("node.name", "null"), ("media.class", "Audio/Sink")]),
            volumes: stereo(1.0),
        },
    );
    graph.defaults.insert(Direction::Output, "null".into());
    assert_eq!(
        graph.volume_target(Direction::Output),
        Some(Target::Node(9))
    );
    let state = graph.state(1);
    assert_eq!(state.output.devices[0].transport, Transport::Virtual);
    // No default: nothing to change, nothing selected.
    graph.defaults.clear();
    assert_eq!(graph.volume_target(Direction::Output), None);
    assert_eq!(graph.state(1).output.volume, None);
    assert!(graph.state(1).output.selected().is_none());
}

#[test]
fn volume_and_balance_as_pulseaudio_reads_them() {
    use Place::{Left, Middle, Right};
    assert_eq!(
        volume_and_balance(&[0.5, 0.5], &[Left, Right]),
        (0.5, Some(0.0))
    );
    assert_eq!(
        volume_and_balance(&[0.5, 0.25], &[Left, Right]),
        (0.5, Some(-0.5))
    );
    assert_eq!(
        volume_and_balance(&[0.25, 0.5], &[Left, Right]),
        (0.5, Some(0.5))
    );
    assert_eq!(
        volume_and_balance(&[0.0, 0.0], &[Left, Right]),
        (0.0, Some(0.0))
    );
    // Mono: no balance.
    assert_eq!(volume_and_balance(&[0.7], &[Middle]), (0.7, None));
    // 5.1: the centre and the subwoofer follow the volume.
    let places = [Left, Right, Middle, Middle, Left, Right];
    let levels = levels_for(0.8, 0.25, &places);
    assert_eq!(
        levels,
        [0.6000000000000001, 0.8, 0.8, 0.8, 0.6000000000000001, 0.8]
    );
    let (volume, balance) = volume_and_balance(&levels, &places);
    assert_eq!(volume, 0.8);
    assert!((balance.unwrap() - 0.25).abs() < 0.001);
    // A volume and balance round trip through PipeWire's linear volumes.
    let volumes = Volumes {
        channels: Vec::new(),
        positions: vec![
            spa::sys::SPA_AUDIO_CHANNEL_FL,
            spa::sys::SPA_AUDIO_CHANNEL_FR,
        ],
        mute: false,
    };
    let channels = volumes.channels_for(0.5, -0.5);
    assert_eq!(channels, [0.125, 0.015625]);
    let volumes = Volumes {
        channels,
        ..volumes
    };
    assert_eq!(volumes.seen(), (0.5, Some(-0.5)));
    // Two channels with no map are a stereo pair.
    let pair = Volumes {
        channels: vec![0.125, 0.015625],
        positions: Vec::new(),
        mute: false,
    };
    assert_eq!(pair.seen(), (0.5, Some(-0.5)));
    assert_eq!(cubic(0.0), 0.0);
    assert_eq!(cubic(1.0), 1.0);
    assert_eq!(linear(0.5), 0.125);
    assert_eq!(linear(2.0), 1.0);
}

#[test]
fn a_meter_shows_the_last_sixty_decibels_in_steps() {
    assert_eq!(meter_level(0.0), 0.0);
    assert_eq!(meter_level(1.0), 1.0);
    assert_eq!(meter_level(2.0), 1.0);
    assert_eq!(meter_level(0.001), 0.0);
    // -20 dB: two thirds, on the steps.
    assert_eq!(meter_level(0.1), 21.0 / 32.0);
    assert_eq!(meter_level(f32::NAN), 0.0);
}

#[test]
fn the_wire_format_round_trips() {
    let value = pipewire_client::props_value(Some(vec![0.125, 0.25]), Some(true));
    let bytes =
        spa::pod::serialize::PodSerializer::serialize(std::io::Cursor::new(Vec::new()), &value)
            .unwrap()
            .0
            .into_inner();
    let pod = spa::pod::Pod::from_bytes(&bytes).unwrap();
    let volumes = pipewire_client::volumes_of(pod).unwrap();
    assert_eq!(volumes.channels, [0.125, 0.25]);
    assert!(volumes.mute);
    // A mute alone has no volumes to read.
    let value = pipewire_client::props_value(None, Some(false));
    let bytes =
        spa::pod::serialize::PodSerializer::serialize(std::io::Cursor::new(Vec::new()), &value)
            .unwrap()
            .0
            .into_inner();
    assert_eq!(
        pipewire_client::volumes_of(spa::pod::Pod::from_bytes(&bytes).unwrap()),
        None
    );
    // A route as a device says it.
    use spa::pod::{Object, Property, Value};
    let route = Value::Object(Object {
        type_: spa::sys::SPA_TYPE_OBJECT_ParamRoute,
        id: spa::sys::SPA_PARAM_Route,
        properties: vec![
            Property::new(spa::sys::SPA_PARAM_ROUTE_index, Value::Int(3)),
            Property::new(
                spa::sys::SPA_PARAM_ROUTE_direction,
                Value::Id(spa::utils::Id(spa::sys::SPA_DIRECTION_OUTPUT)),
            ),
            Property::new(spa::sys::SPA_PARAM_ROUTE_device, Value::Int(1)),
            Property::new(
                spa::sys::SPA_PARAM_ROUTE_name,
                Value::String("hdmi-output-0".into()),
            ),
            Property::new(
                spa::sys::SPA_PARAM_ROUTE_description,
                Value::String("HDMI / DisplayPort".into()),
            ),
            Property::new(
                spa::sys::SPA_PARAM_ROUTE_available,
                Value::Id(spa::utils::Id(spa::sys::SPA_PARAM_AVAILABILITY_yes)),
            ),
            Property::new(
                spa::sys::SPA_PARAM_ROUTE_info,
                Value::Struct(vec![
                    Value::Int(2),
                    Value::String("port.type".into()),
                    Value::String("hdmi".into()),
                    Value::String("device.product.name".into()),
                    Value::String("LG HDR 4K".into()),
                ]),
            ),
            Property::new(
                spa::sys::SPA_PARAM_ROUTE_props,
                pipewire_client::props_value(Some(vec![1.0, 1.0]), Some(false)),
            ),
        ],
    });
    let bytes =
        spa::pod::serialize::PodSerializer::serialize(std::io::Cursor::new(Vec::new()), &route)
            .unwrap()
            .0
            .into_inner();
    let read = pipewire_client::route_of(spa::pod::Pod::from_bytes(&bytes).unwrap()).unwrap();
    assert_eq!(read.index, 3);
    assert_eq!(read.device, 1);
    assert_eq!(read.direction, Some(Direction::Output));
    assert_eq!(read.name, "hdmi-output-0");
    assert_eq!(read.available, Availability::Yes);
    assert_eq!(read.info.get("device.product.name").unwrap(), "LG HDR 4K");
    // What the session sets on a route: its index and sub-device, the props, saved.
    let set = pipewire_client::route_value(3, 1, pipewire_client::props_value(None, Some(true)));
    let Value::Object(object) = set else {
        panic!("an object")
    };
    let keys: Vec<u32> = object.properties.iter().map(|p| p.key).collect();
    assert_eq!(
        keys,
        [
            spa::sys::SPA_PARAM_ROUTE_index,
            spa::sys::SPA_PARAM_ROUTE_device,
            spa::sys::SPA_PARAM_ROUTE_props,
            spa::sys::SPA_PARAM_ROUTE_save
        ]
    );
}

const MACHINE: &str = r#"{
    "outputs": [
        {"id": "speakers", "name": "Speakers", "kind": "speakers", "transport": "builtIn", "volume": 0.5},
        {"id": "dell", "name": "DELL U2720Q", "kind": "hdmi", "transport": "hdmi", "volume": 0.75}
    ],
    "inputs": [
        {"id": "mic", "name": "Microphone", "kind": "microphone", "transport": "builtIn", "volume": 0.8, "balance": null, "level": 0.25}
    ]
}"#;

fn selected(state: &State, direction: Direction) -> Option<String> {
    state.side(direction).selected().map(|d| d.id.clone())
}

#[test]
fn a_played_machine_chooses_as_wireplumber_does() {
    let played = Played::parse(MACHINE).unwrap();
    // Told once, as it starts.
    let events = played.take_events();
    assert_eq!(events.len(), 1);
    let state = played.state();
    assert_eq!(
        selected(&state, Direction::Output).as_deref(),
        Some("speakers")
    );
    assert_eq!(state.output.volume, Some(0.5));
    assert_eq!(state.input.balance, None);
    assert!(played.wake_fd().is_none());

    played.set_default(Direction::Output, "dell");
    assert_eq!(
        selected(&played.state(), Direction::Output).as_deref(),
        Some("dell")
    );
    assert_eq!(
        played.state().output.volume,
        Some(0.75),
        "each device its own volume"
    );
    // Headphones plugged in, chosen; unplugged, the display comes back, as WirePlumber's
    // remembered choices do.
    played
        .act(r#"{"plug": {"direction": "output", "device": {"id": "phones", "name": "Headphones", "kind": "headphones", "transport": "usb"}}}"#)
        .unwrap();
    assert_eq!(
        selected(&played.state(), Direction::Output).as_deref(),
        Some("dell")
    );
    played.set_default(Direction::Output, "phones");
    played.act(r#"{"unplug": {"id": "phones"}}"#).unwrap();
    assert_eq!(
        selected(&played.state(), Direction::Output).as_deref(),
        Some("dell")
    );
    // Plugged again: chosen again.
    played
        .act(r#"{"plug": {"direction": "output", "device": {"id": "phones", "name": "Headphones", "kind": "headphones", "transport": "usb"}}}"#)
        .unwrap();
    assert_eq!(
        selected(&played.state(), Direction::Output).as_deref(),
        Some("phones")
    );
    // Nothing unknown is chosen.
    played.set_default(Direction::Output, "nowhere");
    played.set_volume(Direction::Output, 0.3);
    played.set_balance(0.4);
    played.set_muted(Direction::Output, true);
    let machine = played.machine();
    assert_eq!(machine["default_output"], "phones");
    assert_eq!(
        machine["configured_outputs"],
        serde_json::json!(["phones", "dell"])
    );
    assert_eq!(
        machine["calls"],
        serde_json::json!([
            "default output dell",
            "default output phones",
            "volume output 0.3000030517578125",
            "balance 0.4",
            "muted output true"
        ])
    );
    let state = played.state();
    assert_eq!(state.output.balance, Some(0.4));
    assert!(state.output.muted);
    // Each change told once, as it was.
    let told: Vec<_> = played
        .take_events()
        .into_iter()
        .filter(|event| matches!(event, Event::State(_)))
        .collect();
    assert_eq!(told.len(), 8);
}

#[test]
fn a_played_input_is_metered_only_while_asked() {
    let played = Played::parse(MACHINE).unwrap();
    played.take_events();
    played
        .act(r#"{"change": {"id": "mic", "level": 0.5}}"#)
        .unwrap();
    assert!(played
        .take_events()
        .iter()
        .all(|event| !matches!(event, Event::InputLevel(_))));
    played.meter(true);
    assert_eq!(played.take_events(), [Event::InputLevel(0.5)]);
    // The same level is not told again; a new one is.
    played
        .act(r#"{"change": {"id": "mic", "level": 0.5}}"#)
        .unwrap();
    assert!(played.take_events().is_empty());
    played
        .act(r#"{"change": {"id": "mic", "level": 0.75}}"#)
        .unwrap();
    assert_eq!(played.take_events(), [Event::InputLevel(0.75)]);
    played.meter(false);
    played
        .act(r#"{"change": {"id": "mic", "level": 0.25}}"#)
        .unwrap();
    assert!(played.take_events().is_empty());
    assert_eq!(played.machine()["metering"], false);
    assert!(Played::parse(r#"{"outputs": [{"id": 1}]}"#).is_err());
    assert!(played.act(r#"{"change": {"id": "nothing"}}"#).is_err());
}

#[test]
fn the_state_is_told_as_the_shell_and_settings_read_it() {
    let state = thinkpad().state(1);
    let json = serde_json::to_value(&state).unwrap();
    assert_eq!(json["available"], true);
    assert!(json.get("generation").is_none());
    assert_eq!(
        json["output"]["devices"][1],
        serde_json::json!({
            "id": "alsa_output.hdmi1", "name": "DELL U2720Q", "kind": "hdmi",
            "transport": "hdmi", "selected": false
        })
    );
    assert_eq!(json["output"]["volume"], 0.5);
    assert_eq!(json["input"]["balance"], serde_json::Value::Null);
    assert_eq!(json["input"]["devices"][0]["transport"], "builtIn");
}

#[test]
fn the_processes_heard_are_those_whose_output_streams_run() {
    let mut graph = thinkpad();
    graph.streams.insert(
        90,
        Stream {
            pid: Some(4242),
            running: true,
        },
    );
    // Paused (idle), or of no known process: not heard.
    graph.streams.insert(
        91,
        Stream {
            pid: Some(4243),
            running: false,
        },
    );
    graph.streams.insert(
        92,
        Stream {
            pid: None,
            running: true,
        },
    );
    let state = graph.state(1);
    assert_eq!(state.audible, std::collections::BTreeSet::from([4242]));
    // Settings is not told who plays.
    assert!(serde_json::to_value(&state)
        .unwrap()
        .get("audible")
        .is_none());
    // Nor are streams devices.
    assert_eq!(
        state.output.devices.len(),
        thinkpad().state(1).output.devices.len()
    );
}
