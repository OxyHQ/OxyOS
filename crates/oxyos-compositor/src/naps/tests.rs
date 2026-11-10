//! The applications' states and weights by what the session sees of them, without a scene: what is
//! decided, what is set (and only on a change), when a hidden application naps, and what wakes it.

use super::*;

const SETTINGS: u32 = 100;
const MUSIC: u32 = 101;
const CHAT: u32 = 102;

fn running() -> Vec<(u32, String)> {
    vec![
        (SETTINGS, "so.oxy.settings".to_string()),
        (MUSIC, "so.oxy.music".to_string()),
        (CHAT, "so.oxy.allo".to_string()),
    ]
}

fn cpu(properties: &Properties) -> CpuWeight {
    properties.cpu_weight.unwrap()
}

#[test]
fn each_state_has_its_weights() {
    let at = Instant::now();
    let weights = |activity: Activity| {
        let properties = activity.properties();
        (cpu(&properties), properties.io_weight.unwrap())
    };
    assert_eq!(weights(Activity::Focused), (CpuWeight::Weight(400), 100));
    assert_eq!(weights(Activity::Visible), (CpuWeight::Weight(100), 100));
    assert_eq!(weights(Activity::Busy), (CpuWeight::Weight(100), 100));
    assert_eq!(weights(Activity::Unknown), (CpuWeight::Weight(100), 100));
    assert_eq!(weights(Activity::Hidden(at)), (CpuWeight::Weight(20), 100));
    assert_eq!(weights(Activity::Napping), (CpuWeight::Weight(20), 10));
    assert_eq!(weights(Activity::Background), (CpuWeight::Idle, 10));
}

#[test]
fn a_hidden_application_naps_after_ten_seconds_and_wakes_at_once() {
    let start = Instant::now();
    let hidden = decide(
        Activity::Visible,
        Some(Visibility::Hidden),
        false,
        false,
        start,
    );
    assert_eq!(hidden, Activity::Hidden(start));
    // Still hidden: its time keeps counting from when it was hidden.
    let later = start + Duration::from_secs(9);
    assert_eq!(
        decide(hidden, Some(Visibility::Hidden), false, false, later),
        Activity::Hidden(start)
    );
    let napped = decide(
        hidden,
        Some(Visibility::Hidden),
        false,
        false,
        start + NAP_AFTER,
    );
    assert_eq!(napped, Activity::Napping);
    assert_eq!(
        decide(
            napped,
            Some(Visibility::Hidden),
            false,
            false,
            start + NAP_AFTER * 5
        ),
        Activity::Napping
    );
    // Shown, focused or audible again: awake at once.
    assert_eq!(
        decide(napped, Some(Visibility::Visible), false, false, later),
        Activity::Visible
    );
    assert_eq!(
        decide(napped, Some(Visibility::Focused), false, false, later),
        Activity::Focused
    );
    assert_eq!(
        decide(napped, Some(Visibility::Hidden), false, true, later),
        Activity::Busy
    );
    // Hidden again, it waits its ten seconds again.
    let again = start + Duration::from_secs(30);
    assert_eq!(
        decide(
            Activity::Visible,
            Some(Visibility::Hidden),
            false,
            false,
            again
        ),
        Activity::Hidden(again)
    );
}

#[test]
fn an_audible_or_inhibiting_application_never_naps_and_background_ones_are_idle() {
    let start = Instant::now();
    let mut state = Activity::Visible;
    for second in 0..60 {
        state = decide(
            state,
            Some(Visibility::Hidden),
            false,
            true,
            start + Duration::from_secs(second),
        );
        assert_eq!(state, Activity::Busy);
    }
    assert_eq!(
        decide(Activity::Focused, None, true, false, start),
        Activity::Background
    );
    assert_eq!(
        decide(Activity::Visible, None, false, false, start),
        Activity::Unknown
    );
}

#[test]
fn weights_are_set_only_as_they_change() {
    let mut naps = Naps::new(None);
    let start = Instant::now();
    let none = BTreeSet::new();
    // Nothing shown yet: the defaults stand, nothing is set.
    assert!(naps
        .update(&BTreeMap::new(), &running(), &none, &none, start)
        .is_empty());
    let mut presences = BTreeMap::from([
        (SETTINGS, Visibility::Focused),
        (MUSIC, Visibility::Hidden),
        (CHAT, Visibility::Visible),
    ]);
    let audible = BTreeSet::from([MUSIC]);
    let changes = naps.update(&presences, &running(), &none, &audible, start);
    let set: Vec<(u32, String, CpuWeight)> = changes
        .iter()
        .map(|(pid, unit, properties)| (*pid, unit.name.clone(), cpu(properties)))
        .collect();
    assert_eq!(
        set,
        vec![
            (
                SETTINGS,
                "app-oxyos-so.oxy.settings-100.scope".into(),
                CpuWeight::Weight(400)
            ),
            (
                MUSIC,
                "app-oxyos-so.oxy.music-101.scope".into(),
                CpuWeight::Weight(100)
            ),
            (
                CHAT,
                "app-oxyos-so.oxy.allo-102.scope".into(),
                CpuWeight::Weight(100)
            ),
        ]
    );
    // The same again: nothing to set.
    assert!(naps
        .update(
            &presences,
            &running(),
            &none,
            &audible,
            start + Duration::from_secs(1)
        )
        .is_empty());
    // The chat minimized: hidden at once (20), and no deadline but its nap's.
    presences.insert(CHAT, Visibility::Hidden);
    let changes = naps.update(
        &presences,
        &running(),
        &none,
        &audible,
        start + Duration::from_secs(2),
    );
    assert_eq!(changes.len(), 1);
    assert_eq!(
        (changes[0].0, cpu(&changes[0].2)),
        (CHAT, CpuWeight::Weight(20))
    );
    assert_eq!(
        naps.deadline(),
        Some(start + Duration::from_secs(2) + NAP_AFTER)
    );
    // Still hidden before its nap: nothing; at its nap: its disk's weight too.
    assert!(naps
        .update(
            &presences,
            &running(),
            &none,
            &audible,
            start + Duration::from_secs(11)
        )
        .is_empty());
    let changes = naps.update(
        &presences,
        &running(),
        &none,
        &audible,
        start + Duration::from_secs(12),
    );
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].2.io_weight, Some(10));
    assert_eq!(naps.deadline(), None);
    // Gone: forgotten; the others untouched.
    let fewer: Vec<(u32, String)> = running()
        .into_iter()
        .filter(|(pid, _)| *pid != CHAT)
        .collect();
    assert!(naps
        .update(
            &presences,
            &fewer,
            &none,
            &audible,
            start + Duration::from_secs(13)
        )
        .is_empty());
    assert_eq!(naps.activities().count(), 2);
}

#[test]
fn only_the_focused_application_claims_its_memory_and_the_slice_gets_the_sum() {
    let at = Instant::now();
    let activities = BTreeMap::from([
        (SETTINGS, Activity::Focused),
        (MUSIC, Activity::Visible),
        (CHAT, Activity::Hidden(at)),
        (103, Activity::Napping),
        (104, Activity::Background),
        (105, Activity::Busy),
    ]);
    let usage = BTreeMap::from([
        (SETTINGS, 300 << 20),
        (MUSIC, 900 << 20),
        (CHAT, 200 << 20),
        (103, 100 << 20),
        (104, 50 << 20),
        (105, 400 << 20),
    ]);
    // Its working set now, nobody else's: the hog beside it is protected by nothing.
    let (claims, sum) = super::claims(&activities, &usage, 8 << 30);
    assert_eq!(claims, BTreeMap::from([(SETTINGS, 304 << 20)]));
    assert_eq!(sum, 304 << 20);
    // Rounded up to a step: a little more or less changes nothing.
    let grown = BTreeMap::from([(SETTINGS, (300 << 20) + 4096)]);
    assert_eq!(super::claims(&activities, &grown, 8 << 30).1, 304 << 20);
    // Never more than the cap.
    let (claims, sum) = super::claims(&activities, &usage, 256 << 20);
    assert_eq!(claims, BTreeMap::from([(SETTINGS, 256 << 20)]));
    assert_eq!(sum, 256 << 20);
    // Nothing focused, or not measured: no claim, and the slice protects nothing.
    let none = BTreeMap::from([(MUSIC, Activity::Visible)]);
    assert_eq!(super::claims(&none, &usage, 8 << 30), (BTreeMap::new(), 0));
    assert_eq!(
        super::claims(&activities, &BTreeMap::new(), 8 << 30),
        (BTreeMap::new(), 0)
    );
}
