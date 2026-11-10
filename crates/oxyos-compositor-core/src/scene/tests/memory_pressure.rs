//! Memory pressure (docs/adr/0066-quality-of-service.mdx, decision 7): a warning to the
//! applications nobody sees, critical to every application and the shell, each told a level once
//! and normal again once it eases.

use super::*;
use oxyos_protocol::{MemoryPressureLevel, MEMORY_PRESSURE_VERSION};

const HIDDEN: ClientId = 30;
const OLD: ClientId = 31;
const WINDOWLESS: ClientId = 32;

fn told(events: &[(ClientId, Event)]) -> Vec<(ClientId, MemoryPressureLevel)> {
    events
        .iter()
        .filter_map(|(client, event)| match event {
            Event::MemoryPressure { level } => Some((*client, *level)),
            _ => None,
        })
        .collect()
}

/// APP shown; HIDDEN minimized; WINDOWLESS an application running without a window (ADR 0060);
/// OLD an application of the version before; OTHER a client no launch named.
fn harness() -> Harness {
    let mut h = Harness::new();
    for client in [HIDDEN, OLD, WINDOWLESS] {
        h.scene.connect(client, Grants::default()).unwrap();
    }
    h.scene.set_version(OLD, MEMORY_PRESSURE_VERSION - 1);
    for (client, id) in [
        (APP, "so.oxy.allo"),
        (HIDDEN, "so.oxy.inbox"),
        (OLD, "so.oxy.old"),
        (WINDOWLESS, "so.oxy.chat"),
    ] {
        h.scene.set_application(client, id);
    }
    h.toplevel(APP, 1, 400, 300);
    let hidden = h.toplevel(HIDDEN, 1, 400, 300);
    h.toplevel(OLD, 1, 400, 300);
    h.scene.minimize(hidden);
    h.scene.minimize(key(OLD, 1));
    logged(&mut h);
    h
}

#[test]
fn a_warning_tells_the_applications_nobody_sees_and_critical_every_one_and_the_shell() {
    let mut h = harness();
    // Nothing is told while nothing was said.
    h.scene.tell_memory_pressure();
    assert!(told(&logged(&mut h)).is_empty());
    h.scene.set_memory_pressure(MemoryPressureLevel::Warning);
    assert_eq!(
        told(&logged(&mut h)),
        [
            (HIDDEN, MemoryPressureLevel::Warning),
            (WINDOWLESS, MemoryPressureLevel::Warning)
        ]
    );
    // Said again, or every turn, nothing more.
    h.scene.set_memory_pressure(MemoryPressureLevel::Warning);
    h.scene.tell_memory_pressure();
    assert!(told(&logged(&mut h)).is_empty());
    // Critical: every application and the shell (no window of its own is a toplevel), those not
    // shown first; the old one never.
    h.scene.set_memory_pressure(MemoryPressureLevel::Critical);
    let critical = told(&logged(&mut h));
    assert!(critical
        .iter()
        .all(|(_, level)| *level == MemoryPressureLevel::Critical));
    let order: Vec<ClientId> = critical.iter().map(|(client, _)| *client).collect();
    assert_eq!(order, [SHELL, HIDDEN, WINDOWLESS, APP]);
    // Lower again while it lasts: what each gave back stays given back, nobody is told.
    h.scene.set_memory_pressure(MemoryPressureLevel::Warning);
    assert!(told(&logged(&mut h)).is_empty());
    // Eased: each told is told once.
    h.scene.set_memory_pressure(MemoryPressureLevel::Normal);
    let eased = told(&logged(&mut h));
    assert_eq!(eased.len(), 4);
    assert!(eased
        .iter()
        .all(|(_, level)| *level == MemoryPressureLevel::Normal));
    h.scene.tell_memory_pressure();
    assert!(told(&logged(&mut h)).is_empty());
    assert!(to(&logged(&mut h), OTHER).is_empty());
}

#[test]
fn an_application_hidden_while_memory_is_short_is_told_then_and_one_gone_forgotten() {
    let mut h = harness();
    h.scene.set_memory_pressure(MemoryPressureLevel::Warning);
    logged(&mut h);
    // The shown application minimized: told at the program's next turn.
    h.scene.minimize(key(APP, 1));
    h.scene.tell_memory_pressure();
    assert_eq!(
        told(&logged(&mut h)),
        [(APP, MemoryPressureLevel::Warning)]
    );
    // One that goes is not told it eased.
    h.scene.disconnect(HIDDEN);
    h.scene.set_memory_pressure(MemoryPressureLevel::Normal);
    let eased: Vec<ClientId> = told(&logged(&mut h)).iter().map(|(c, _)| *c).collect();
    assert_eq!(eased, [APP, WINDOWLESS]);
}

#[test]
fn the_lock_hides_every_window() {
    let mut h = harness();
    h.scene.lock();
    logged(&mut h);
    h.scene.set_memory_pressure(MemoryPressureLevel::Warning);
    let warned: Vec<ClientId> = told(&logged(&mut h)).iter().map(|(c, _)| *c).collect();
    assert_eq!(warned, [APP, HIDDEN, WINDOWLESS]);
}
