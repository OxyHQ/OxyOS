use crate::model::{Interfaces, Objects, ADAPTER, BATTERY, DEVICE};
use crate::*;
use std::collections::{BTreeMap, HashMap};
use zbus::zvariant::{OwnedValue, Value};

fn owned(value: Value<'static>) -> OwnedValue {
    OwnedValue::try_from(value).unwrap()
}

fn properties(pairs: Vec<(&str, Value<'static>)>) -> HashMap<String, OwnedValue> {
    pairs
        .into_iter()
        .map(|(key, value)| (key.to_string(), owned(value)))
        .collect()
}

fn adapter(powered: bool) -> Interfaces {
    let mut interfaces = HashMap::new();
    interfaces.insert(
        ADAPTER.to_string(),
        properties(vec![
            ("Address", Value::from("00:1A:7D:DA:71:13")),
            ("Name", Value::from("thinkpad")),
            ("Alias", Value::from("ThinkPad")),
            ("Powered", Value::from(powered)),
            (
                "PowerState",
                Value::from(if powered { "on" } else { "off" }),
            ),
        ]),
    );
    interfaces
}

fn device(address: &str, name: Option<&str>, icon: &str, paired: bool) -> Interfaces {
    let mut pairs = vec![
        ("Address", Value::from(address.to_string())),
        (
            "Adapter",
            Value::from(zbus::zvariant::ObjectPath::from_static_str_unchecked(
                "/org/bluez/hci0",
            )),
        ),
        (
            "Alias",
            Value::from(name.map_or_else(|| address.replace(':', "-"), str::to_string)),
        ),
        ("Icon", Value::from(icon.to_string())),
        ("Paired", Value::from(paired)),
        ("Connected", Value::from(false)),
    ];
    if let Some(name) = name {
        pairs.push(("Name", Value::from(name.to_string())));
    }
    let mut interfaces = HashMap::new();
    interfaces.insert(DEVICE.to_string(), properties(pairs));
    interfaces
}

fn path(address: &str) -> String {
    format!("/org/bluez/hci0/dev_{}", address.replace(':', "_"))
}

fn machine() -> Objects {
    let mut objects = Objects::default();
    objects.added("/org/bluez/hci0", &adapter(true));
    for (address, name, icon, paired) in [
        (
            "AC:90:85:11:22:33",
            Some("AirPods Pro"),
            "audio-headphones",
            true,
        ),
        (
            "F0:B3:EC:44:55:66",
            Some("magic Keyboard"),
            "input-keyboard",
            true,
        ),
        ("3C:28:6D:01:02:03", Some("Pixel 8"), "phone", false),
        ("5A:11:22:33:44:55", None, "", false),
        ("5A:11:22:33:44:56", None, "input-mouse", true),
    ] {
        objects.added(&path(address), &device(address, name, icon, paired));
    }
    objects
}

#[test]
fn kinds_from_bluez_icons_and_services() {
    assert_eq!(kind(Some("audio-headset"), &[]), Kind::Headphones);
    assert_eq!(kind(Some("audio-headphones"), &[]), Kind::Headphones);
    assert_eq!(kind(Some("audio-card"), &[]), Kind::Speaker);
    assert_eq!(kind(Some("input-keyboard"), &[]), Kind::Keyboard);
    assert_eq!(kind(Some("input-mouse"), &[]), Kind::Mouse);
    // BlueZ names a pointing digitizer (a Magic Trackpad's class) a tablet.
    assert_eq!(kind(Some("input-tablet"), &[]), Kind::Trackpad);
    assert_eq!(kind(Some("input-gaming"), &[]), Kind::Gamepad);
    assert_eq!(kind(Some("phone"), &[]), Kind::Phone);
    assert_eq!(kind(Some("computer"), &[]), Kind::Computer);
    assert_eq!(kind(Some("printer"), &[]), Kind::Other);
    let a2dp = vec!["0000110B-0000-1000-8000-00805F9B34FB".to_string()];
    assert_eq!(kind(None, &a2dp), Kind::Headphones);
    assert!(is_audio(Kind::Other, &a2dp));
    let hogp = vec!["00001812-0000-1000-8000-00805f9b34fb".to_string()];
    assert!(is_input(Kind::Other, &hogp) && !is_audio(Kind::Other, &hogp));
    assert!(!is_audio(
        Kind::Other,
        &["12345678-0000-1000-8000-00805f9b34fb".into()]
    ));
    assert_eq!(service("0000110d-0000-1000-8000-00805f9b34fb"), "audio");
    assert_eq!(service("00001105-0000-1000-8000-00805f9b34fb"), "files");
    assert_eq!(service("0000112f-0000-1000-8000-00805f9b34fb"), "phone");
    assert_eq!(service("00001116-0000-1000-8000-00805f9b34fb"), "network");
    assert_eq!(service("not a uuid"), "other");
}

#[test]
fn the_state_shows_paired_devices_and_named_ones_by_name() {
    let objects = machine();
    let state = objects.state(true, &BTreeMap::new());
    let adapter = state.adapter.as_ref().unwrap();
    assert_eq!(adapter.name, "ThinkPad");
    assert!(adapter.powered && !adapter.blocked);
    let shown: Vec<(&str, bool)> = state
        .devices
        .iter()
        .map(|d| (d.name.as_str(), d.paired))
        .collect();
    // Case does not order them; a paired device with no name shows its address; an unnamed one
    // nearby is not shown.
    assert_eq!(
        shown,
        [
            ("5A:11:22:33:44:56", true),
            ("AirPods Pro", true),
            ("magic Keyboard", true),
            ("Pixel 8", false)
        ]
    );
    assert!(State::unavailable() == objects.state(false, &BTreeMap::new()));
    assert_eq!(
        Objects::default().state(true, &BTreeMap::new()),
        State {
            available: true,
            ..State::default()
        }
    );
}

#[test]
fn signals_change_the_state_and_only_what_is_shown_tells() {
    let mut objects = machine();
    let before = objects.state(true, &BTreeMap::new());
    let airpods = path("AC:90:85:11:22:33");
    // A discovery's signal strength changes nothing shown.
    objects.changed(
        &airpods,
        DEVICE,
        &properties(vec![("RSSI", Value::from(-40i16))]),
        &[],
    );
    assert_eq!(objects.state(true, &BTreeMap::new()), before);
    objects.changed(
        &airpods,
        DEVICE,
        &properties(vec![("Connected", Value::from(true))]),
        &[],
    );
    let mut battery = HashMap::new();
    battery.insert(
        BATTERY.to_string(),
        properties(vec![("Percentage", Value::from(80u8))]),
    );
    objects.added(&airpods, &battery);
    let mut busy = BTreeMap::new();
    busy.insert("F0:B3:EC:44:55:66".to_string(), Busy::Connecting);
    let state = objects.state(true, &busy);
    let shown = state.device("AC:90:85:11:22:33").unwrap();
    assert!(shown.connected && shown.audio);
    assert_eq!(shown.battery, Some(80));
    assert_eq!(
        state.device("F0:B3:EC:44:55:66").unwrap().busy,
        Some(Busy::Connecting)
    );
    assert_eq!(state.connected(), 1);
    // Renamed: its alias; its battery gone with its interface.
    objects.changed(
        &airpods,
        DEVICE,
        &properties(vec![("Alias", Value::from("Nate's AirPods"))]),
        &[],
    );
    objects.removed(&airpods, &[BATTERY.to_string()]);
    let state = objects.state(true, &BTreeMap::new());
    let shown = state.device("AC:90:85:11:22:33").unwrap();
    assert_eq!(
        (shown.name.as_str(), shown.battery),
        ("Nate's AirPods", None)
    );
    // Blocked by rfkill; the adapter gone takes its devices.
    objects.changed(
        "/org/bluez/hci0",
        ADAPTER,
        &properties(vec![
            ("Powered", Value::from(false)),
            ("PowerState", Value::from("off-blocked")),
        ]),
        &[],
    );
    assert!(objects
        .state(true, &BTreeMap::new())
        .adapter
        .is_some_and(|a| a.blocked && !a.powered));
    objects.removed("/org/bluez/hci0", &[ADAPTER.to_string()]);
    let state = objects.state(true, &BTreeMap::new());
    assert!(state.adapter.is_none() && state.devices.is_empty());
}

#[test]
fn the_state_as_told() {
    let mut objects = Objects::default();
    objects.added("/org/bluez/hci0", &adapter(true));
    objects.added(
        &path("AC:90:85:11:22:33"),
        &device(
            "AC:90:85:11:22:33",
            Some("AirPods Pro"),
            "audio-headphones",
            true,
        ),
    );
    let text = serde_json::to_value(objects.state(true, &BTreeMap::new())).unwrap();
    assert_eq!(
        text,
        serde_json::json!({
            "available": true,
            "adapter": {"name": "ThinkPad", "address": "00:1A:7D:DA:71:13", "powered": true,
                        "blocked": false, "discovering": false, "discoverable": false},
            "devices": [{"id": "AC:90:85:11:22:33", "address": "AC:90:85:11:22:33",
                         "name": "AirPods Pro", "kind": "headphones", "paired": true,
                         "connected": false, "busy": null, "battery": null, "audio": true,
                         "input": false}],
        })
    );
}

#[test]
fn names_codes_and_failures() {
    assert_eq!(name_given("  Nate's Mouse \n"), "Nate's Mouse");
    let long = "é".repeat(200);
    let cut = name_given(&long);
    assert!(cut.len() <= NAME_LIMIT && cut.chars().all(|c| c == 'é'));
    assert!(code_valid(&Ask::PinCode, "0000"));
    assert!(code_valid(&Ask::PinCode, "1234567890123456"));
    assert!(!code_valid(&Ask::PinCode, "12345678901234567"));
    assert!(!code_valid(&Ask::PinCode, ""));
    assert!(code_valid(&Ask::Passkey, "482913"));
    assert!(code_valid(&Ask::Passkey, "7302"));
    assert!(!code_valid(&Ask::Passkey, "1234567"));
    assert!(!code_valid(&Ask::Passkey, "12a4"));
    assert!(!code_valid(&Ask::Confirm(1), "1"));
    assert_eq!(
        Failure::of("org.bluez.Error.AuthenticationCanceled", ""),
        Failure::Cancelled
    );
    assert_eq!(
        Failure::of("org.bluez.Error.AuthenticationFailed", ""),
        Failure::Rejected
    );
    assert_eq!(
        Failure::of("org.bluez.Error.Failed", "br-connection-page-timeout"),
        Failure::Timeout
    );
    assert_eq!(
        Failure::of("org.bluez.Error.Failed", "Blocked through rfkill"),
        Failure::Blocked
    );
    assert_eq!(Failure::of("org.bluez.Error.InProgress", ""), Failure::Busy);
    assert_eq!(
        Failure::of("org.bluez.Error.NotReady", ""),
        Failure::Unavailable
    );
    assert_eq!(
        Failure::of("org.bluez.Error.Failed", "anything"),
        Failure::Failed
    );
    assert_eq!(Failure::Rejected.reason(), "rejected");
}

#[test]
fn a_question_dropped_unanswered_is_refused() {
    let (reply, answered) = async_channel::bounded(1);
    drop(Question {
        id: 1,
        ask: Ask::Authorize,
        address: String::new(),
        name: String::new(),
        kind: Kind::Other,
        reply: Some(reply),
    });
    assert_eq!(answered.try_recv(), Ok(Answer::Refuse));
}
