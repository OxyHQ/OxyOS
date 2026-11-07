//! The session's connection to BlueZ (`oxyos_bluetooth::run`, docs/adr/0050-bluetooth.mdx) against
//! the played BlueZ on a bus of the test's own: what it reads and follows, discovery held and
//! ended, the pairing agent's every kind of question, connecting, disconnecting, forgetting,
//! renaming, and what it leaves behind as it stops.

mod bluetooth;

use bluetooth::{Played, Session, MACHINE};
use oxyos_bluetooth::{Answer, Ask, Busy, Command, Done, Event, Failure, Kind};

#[test]
fn reads_the_machine_and_registers_the_agent() {
    let played = Played::start("read", MACHINE);
    let session = Session::start(&played);
    let state = &session.state;
    let adapter = state.adapter.as_ref().unwrap();
    assert_eq!(adapter.name, "ThinkPad");
    assert!(adapter.powered && !adapter.blocked && !adapter.discovering);
    // The paired devices alone, by name: nothing nearby without a discovery.
    let names: Vec<&str> = state.devices.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(names, ["AirPods Pro", "Magic Keyboard", "MX Master 3S"]);
    let airpods = state.device("AC:90:85:11:22:33").unwrap();
    assert_eq!(airpods.kind, Kind::Headphones);
    assert!(airpods.paired && airpods.connected && airpods.audio && !airpods.input);
    assert_eq!(airpods.battery, Some(80));
    let keyboard = state.device("F0:B3:EC:44:55:66").unwrap();
    assert_eq!(
        (keyboard.kind, keyboard.input, keyboard.connected),
        (Kind::Keyboard, true, false)
    );
    assert_eq!(state.connected(), 2);
    played.until("the agent, the default", |s| {
        s["agent"]["default"] == true && s["agent"]["capability"] == "KeyboardDisplay"
    });
    assert!(session.stop() < std::time::Duration::from_secs(1));
    played.until("the agent gone", |s| s["agent"].is_null());
}

#[test]
fn discovery_is_held_while_wanted_and_ended_with_the_session() {
    let played = Played::start("discover", MACHINE);
    let mut session = Session::start(&played);
    session.send(Command::Discover(true));
    session.until_state("the nearby devices", |s| {
        s.adapter
            .as_ref()
            .is_some_and(|a| a.discovering && a.discoverable)
            && s.devices.iter().filter(|d| !d.paired).count() == 5
    });
    // A device with no name of its own is not shown; the others by name.
    assert!(session.state.device("5A:11:22:33:44:55").is_none());
    played.until("one discovery, filtered", |s| {
        s["adapter"]["discovery_clients"] == 1 && s["adapter"]["filtered"] == true
    });
    session.send(Command::Discover(false));
    session.until_state("the discovery ended", |s| {
        s.adapter
            .as_ref()
            .is_some_and(|a| !a.discovering && !a.discoverable)
            && s.devices.iter().all(|d| d.paired)
    });
    // Held again, the session ending ends it: nothing is left discovering or discoverable.
    session.send(Command::Discover(true));
    session.until_state("discovering again", |s| {
        s.adapter.as_ref().is_some_and(|a| a.discovering)
    });
    session.stop();
    played.until("nothing left discovering", |s| {
        s["adapter"]["discovering"] == false && s["adapter"]["discoverable"] == false
    });
}

#[test]
fn turning_off_ends_discovery_and_on_again_takes_it_up() {
    let played = Played::start("power", MACHINE);
    let mut session = Session::start(&played);
    session.send(Command::Discover(true));
    session.until_state("discovering", |s| {
        s.adapter.as_ref().is_some_and(|a| a.discovering)
    });
    let call = session.call();
    session.send(Command::Power { on: false, call });
    assert_eq!(session.done(call), Ok(Done::Powered(false)));
    session.until_state("off", |s| {
        s.adapter
            .as_ref()
            .is_some_and(|a| !a.powered && !a.discovering)
            && s.connected() == 0
    });
    let call = session.call();
    session.send(Command::Power { on: true, call });
    assert_eq!(session.done(call), Ok(Done::Powered(true)));
    session.until_state("discovering again", |s| {
        s.adapter
            .as_ref()
            .is_some_and(|a| a.powered && a.discovering)
    });
    // Blocked by rfkill: said so, and it cannot be turned on.
    played.act(r#"{"blocked": true}"#);
    session.until_state("blocked", |s| {
        s.adapter.as_ref().is_some_and(|a| a.blocked && !a.powered)
    });
    let call = session.call();
    session.send(Command::Power { on: true, call });
    assert_eq!(session.done(call), Err(Failure::Blocked));
}

/// Connects `id`, answering the agent's one question with `answer`; the question and the outcome.
fn connect_answering(
    session: &mut Session,
    id: &str,
    answer: Answer,
) -> (Ask, Result<Done, Failure>) {
    let call = session.call();
    session.send(Command::Connect {
        id: id.into(),
        call,
    });
    let question = session.question();
    let ask = question.ask.clone();
    assert_eq!(question.address, id);
    question.answer(answer);
    (ask, session.done(call))
}

#[test]
fn pairing_asks_what_each_device_needs() {
    let played = Played::start("pairing", MACHINE);
    let mut session = Session::start(&played);
    session.send(Command::Discover(true));
    session.until_state("nearby", |s| s.devices.len() == 8);

    // Numeric comparison, refused by the person: cancelled, nothing paired.
    let (ask, outcome) = connect_answering(&mut session, "3C:28:6D:01:02:03", Answer::Refuse);
    assert_eq!(ask, Ask::Confirm(482913));
    assert_eq!(outcome, Err(Failure::Cancelled));
    assert!(!session.state.device("3C:28:6D:01:02:03").unwrap().paired);
    // Confirmed: paired and trusted; a phone has nothing to connect, so paired is all it is.
    let (_, outcome) = connect_answering(&mut session, "3C:28:6D:01:02:03", Answer::Accept);
    assert_eq!(outcome, Ok(Done::Paired));
    played.until("the phone paired and trusted", |s| {
        let phone = &s["devices"]["3C:28:6D:01:02:03"];
        phone["paired"] == true && phone["trusted"] == true
    });

    // A legacy PIN: a wrong one is the other device's refusal, the right one pairs and connects.
    let (ask, outcome) = connect_answering(
        &mut session,
        "00:0D:44:AA:BB:CC",
        Answer::Code("0000".into()),
    );
    assert_eq!(ask, Ask::PinCode);
    assert_eq!(outcome, Err(Failure::Rejected));
    let (_, outcome) = connect_answering(
        &mut session,
        "00:0D:44:AA:BB:CC",
        Answer::Code("1234".into()),
    );
    assert_eq!(outcome, Ok(Done::Connected));
    session.until_state("the car kit connected", |s| {
        s.device("00:0D:44:AA:BB:CC")
            .is_some_and(|d| d.paired && d.connected && d.busy.is_none())
    });

    // Just works: nothing asked.
    let call = session.call();
    session.send(Command::Connect {
        id: "B8:F6:53:0D:0E:0F".into(),
        call,
    });
    assert_eq!(session.done(call), Ok(Done::Connected));

    // A passkey shown to type on the keyboard: the question ends as it is typed.
    let call = session.call();
    session.send(Command::Connect {
        id: "00:1F:20:0A:0B:0C".into(),
        call,
    });
    let shown = session.question();
    assert_eq!(shown.ask, Ask::ShowPasskey(7302));
    let id = shown.id;
    drop(shown);
    session.until_state("connecting", |s| {
        s.device("00:1F:20:0A:0B:0C")
            .is_some_and(|d| d.busy == Some(Busy::Connecting))
    });
    played.act(r#"{"typed": "00:1F:20:0A:0B:0C"}"#);
    let mut ended = false;
    loop {
        match session.next() {
            Event::QuestionEnded(ended_id) => {
                assert_eq!(ended_id, id);
                ended = true;
            }
            Event::Done {
                call: done,
                outcome,
            } if done == call => {
                assert_eq!(outcome, Ok(Done::Connected));
                break;
            }
            Event::State(_) => {}
            other => panic!("{other:?}"),
        }
    }
    assert!(ended, "the code shown ended with the pairing");
}

#[test]
fn a_code_shown_and_cancelled_ends_the_pairing() {
    let played = Played::start("cancel", MACHINE);
    let mut session = Session::start(&played);
    session.send(Command::Discover(true));
    session.until_state("nearby", |s| s.devices.len() == 8);
    let call = session.call();
    session.send(Command::Connect {
        id: "00:1F:20:0A:0B:0C".into(),
        call,
    });
    let shown = session.question();
    assert_eq!(shown.ask, Ask::ShowPasskey(7302));
    // The person cancels the sheet showing it: the pairing is cancelled.
    shown.answer(Answer::Refuse);
    session.send(Command::CancelPairing {
        id: "00:1F:20:0A:0B:0C".into(),
    });
    let outcome = session.done(call);
    assert!(
        matches!(outcome, Err(Failure::Cancelled)),
        "a pairing the person cancelled: {outcome:?}"
    );
    // BlueZ cancelling a question it asked ends it.
    let call = session.call();
    session.send(Command::Connect {
        id: "3C:28:6D:01:02:03".into(),
        call,
    });
    let asked = session.question();
    let id = asked.id;
    played.act(r#"{"cancel": "3C:28:6D:01:02:03"}"#);
    loop {
        match session.next() {
            Event::QuestionEnded(ended) => {
                assert_eq!(ended, id);
                break;
            }
            Event::State(_) => {}
            other => panic!("{other:?}"),
        }
    }
    drop(asked);
    assert!(session.done(call).is_err());
}

#[test]
fn a_device_that_asks_to_pair_or_for_a_service() {
    let played = Played::start("asked", MACHINE);
    let mut session = Session::start(&played);
    session.send(Command::Discover(true));
    session.until_state("nearby", |s| s.devices.len() == 8);
    played.act(r#"{"pair_request": "3C:28:6D:01:02:03"}"#);
    let question = session.question();
    assert_eq!(question.ask, Ask::Confirm(482913));
    assert_eq!(
        (question.name.as_str(), question.kind),
        ("Pixel 8", Kind::Phone)
    );
    question.answer(Answer::Accept);
    session.until_state("the phone paired", |s| {
        s.device("3C:28:6D:01:02:03").is_some_and(|d| d.paired)
    });
    played.act(
        r#"{"service_request": {"address": "F0:B3:EC:44:55:66", "uuid": "00001124-0000-1000-8000-00805f9b34fb"}}"#,
    );
    let question = session.question();
    assert_eq!(
        question.ask,
        Ask::Service("00001124-0000-1000-8000-00805f9b34fb".into())
    );
    assert_eq!(
        oxyos_bluetooth::service("00001124-0000-1000-8000-00805f9b34fb"),
        "input"
    );
    // Unanswered (dropped), it is refused.
    drop(question);
    played.until("the service refused", |s| {
        s["log"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l == "service request F0:B3:EC:44:55:66: refused")
    });
}

#[test]
fn connecting_disconnecting_forgetting_and_renaming() {
    let played = Played::start("devices", MACHINE);
    let mut session = Session::start(&played);
    // A paired keyboard connects, and its battery shows as it reports one.
    let call = session.call();
    session.send(Command::Connect {
        id: "F0:B3:EC:44:55:66".into(),
        call,
    });
    assert_eq!(session.done(call), Ok(Done::Connected));
    played.act(r#"{"battery": {"address": "F0:B3:EC:44:55:66", "percent": 64}}"#);
    session.until_state("the keyboard's battery", |s| {
        s.device("F0:B3:EC:44:55:66")
            .is_some_and(|d| d.connected && d.battery == Some(64))
    });
    let call = session.call();
    session.send(Command::Disconnect {
        id: "AC:90:85:11:22:33".into(),
        call,
    });
    assert_eq!(session.done(call), Ok(Done::Disconnected));
    session.until_state("the AirPods disconnected", |s| {
        s.device("AC:90:85:11:22:33").is_some_and(|d| !d.connected)
    });
    // Renamed, and an empty name gives it its own back.
    let call = session.call();
    session.send(Command::Rename {
        id: "D4:5A:66:77:88:99".into(),
        name: "  Nate's Mouse  ".into(),
        call,
    });
    assert_eq!(session.done(call), Ok(Done::Renamed("Nate's Mouse".into())));
    session.until_state("renamed", |s| {
        s.device("D4:5A:66:77:88:99")
            .is_some_and(|d| d.name == "Nate's Mouse")
    });
    let call = session.call();
    session.send(Command::Rename {
        id: "D4:5A:66:77:88:99".into(),
        name: String::new(),
        call,
    });
    assert_eq!(session.done(call), Ok(Done::Renamed(String::new())));
    session.until_state("its own name back", |s| {
        s.device("D4:5A:66:77:88:99")
            .is_some_and(|d| d.name == "MX Master 3S")
    });
    let call = session.call();
    session.send(Command::Forget {
        id: "F0:B3:EC:44:55:66".into(),
        call,
    });
    assert_eq!(session.done(call), Ok(Done::Forgotten));
    session.until_state("forgotten", |s| s.device("F0:B3:EC:44:55:66").is_none());
    // A device that is not there, and one that does not answer.
    let call = session.call();
    session.send(Command::Disconnect {
        id: "11:22:33:44:55:66".into(),
        call,
    });
    assert_eq!(session.done(call), Err(Failure::NotFound));
    session.send(Command::Discover(true));
    session.until_state("nearby", |s| s.device("70:BF:92:DD:EE:FF").is_some());
    let call = session.call();
    session.send(Command::Connect {
        id: "70:BF:92:DD:EE:FF".into(),
        call,
    });
    assert_eq!(session.done(call), Err(Failure::Timeout));
}
