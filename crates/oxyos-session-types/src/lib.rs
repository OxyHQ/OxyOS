//! What the session's services and the display protocol both name: a level the session controls
//! and the battery, Wi-Fi and Bluetooth as it finds them (ADR 0025), the system's sounds (ADR 0044)
//! and what an assistive technology is told of a node (ADR 0003, ADR 0014).
//!
//! The services (`oxyos-controls`, `oxyos-sound`, `oxyos-speech`) use these without the display
//! protocol; `oxyos-protocol` carries them on the wire (its own encoding, the same values) and
//! re-exports them, so its users name them as before. Nothing here does I/O
//! (docs/native-architecture.mdx, Crate layers: leaf).

/// Closed sets of values with their numbers: the protocol encodes each as its `u8`, and a number
/// outside the set is no value (`from_raw`).
macro_rules! enumeration {
    ($(#[$meta:meta])* $name:ident: $repr:ty { $($variant:ident = $value:literal),* $(,)? }) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        #[repr($repr)]
        pub enum $name { $($variant = $value),* }

        impl $name {
            /// The variant with this wire value, if there is one.
            pub fn from_raw(value: $repr) -> Option<Self> {
                match value {
                    $($value => Some(Self::$variant),)*
                    _ => None,
                }
            }
        }
    };
}

/// A whole level of a control (`ControlLevel`, `SetControlLevel`, ADR 0025): 65536 is all of it.
pub const FULL_LEVEL: u32 = 65_536;

enumeration! {
    /// A sound of the system's (docs/adr/0044-system-sounds.mdx), by what it means: items put in
    /// the Trash, the Trash emptied, a notification arriving. The session plays each from its
    /// sound theme; an application never sends one.
    SystemSound: u8 { MoveToTrash = 1, EmptyTrash = 2, Notification = 3 }
}
enumeration! {
    /// What an accessible node is, for an assistive technology (ADR 0003). Unknown roles are
    /// published as `Generic`, never guessed. `ColumnHeader` is version 14's (ADR 0039): a
    /// table's or a list's column header, never sent to a peer of version 13 or older. `Meter`
    /// is version 15's: a level in a known range that is no progress (a battery's charge, ARIA's
    /// `meter`, VoiceOver's level indicator), never sent to a peer of version 14 or older.
    AccessibleRole: u8 {
        Generic = 0, Button = 1, Link = 2, Text = 3, Heading = 4, Image = 5, TextInput = 6,
        Menu = 7, MenuItem = 8, CheckBox = 9, Radio = 10, Switch = 11, Tab = 12, TabList = 13,
        List = 14, ListItem = 15, Dialog = 16, Alert = 17, ProgressBar = 18, Slider = 19,
        Search = 20, Toolbar = 21, ScrollView = 22, Group = 23, ComboBox = 24, Summary = 25,
        Grid = 26, ColumnHeader = 27, Meter = 28,
    }
}
enumeration! {
    /// How an announcement interrupts an assistive technology: after what it is saying
    /// (`Polite`), or at once (`Assertive`), as Android's live regions.
    Politeness: u8 { Polite = 1, Assertive = 2 }
}
enumeration! {
    /// A level the session controls (ADR 0025): the volume of the default output and of the
    /// default input (the microphone), and the backlight of the built-in display and of the
    /// keyboard.
    Control: u8 { OutputVolume = 1, InputVolume = 2, DisplayBrightness = 3, KeyboardBrightness = 4 }
}
enumeration! {
    /// The battery, as the session finds it (ADR 0025): none, discharging, charging, full, or on
    /// AC power and not charging.
    BatteryState: u8 { Absent = 0, Discharging = 1, Charging = 2, Full = 3, NotCharging = 4 }
}
enumeration! {
    /// Wi-Fi, as the session finds it (ADR 0025): no Wi-Fi device (or nothing managing one),
    /// turned off, on and not connected, connecting, connected.
    WifiState: u8 { Unavailable = 0, Off = 1, Disconnected = 2, Connecting = 3, Connected = 4 }
}
enumeration! {
    /// Bluetooth, as the session finds it (ADR 0025): no adapter (or nothing managing one), its
    /// adapter off, or on.
    BluetoothState: u8 { Unavailable = 0, Off = 1, On = 2 }
}

/// An accessible node's states, as bits. Unknown bits are invalid.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct AccessibleState(u16);

impl AccessibleState {
    pub const DISABLED: u16 = 1 << 0;
    pub const SELECTED: u16 = 1 << 1;
    pub const CHECKED: u16 = 1 << 2;
    /// Checked and unchecked at once (a partly checked group).
    pub const MIXED: u16 = 1 << 3;
    pub const EXPANDED: u16 = 1 << 4;
    /// Expandable and not expanded.
    pub const COLLAPSED: u16 = 1 << 5;
    pub const BUSY: u16 = 1 << 6;
    /// The node with the application's accessibility focus.
    pub const FOCUSED: u16 = 1 << 7;
    /// A live region (Android's `accessibilityLiveRegion`): an assistive technology announces
    /// what changes in it, after what it is saying or at once.
    pub const LIVE_POLITE: u16 = 1 << 8;
    pub const LIVE_ASSERTIVE: u16 = 1 << 9;
    const ALL: u16 = (1 << 10) - 1;

    /// The state with these bits, if every bit is known.
    pub const fn new(bits: u16) -> Option<Self> {
        if bits & !Self::ALL == 0 {
            Some(Self(bits))
        } else {
            None
        }
    }

    pub const fn bits(self) -> u16 {
        self.0
    }

    pub const fn contains(self, bit: u16) -> bool {
        self.0 & bit == bit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_value_is_its_number_and_no_other_number_is_a_value() {
        for raw in 0..=u8::MAX {
            if let Some(role) = AccessibleRole::from_raw(raw) {
                assert_eq!(role as u8, raw);
            }
            if let Some(state) = BatteryState::from_raw(raw) {
                assert_eq!(state as u8, raw);
            }
        }
        assert_eq!(AccessibleRole::from_raw(29), None);
        assert_eq!(SystemSound::from_raw(0), None);
        assert_eq!(Control::from_raw(5), None);
        assert_eq!(WifiState::from_raw(5), None);
        assert_eq!(BluetoothState::from_raw(3), None);
        assert_eq!(Politeness::from_raw(3), None);
    }

    #[test]
    fn an_accessible_state_has_only_known_bits() {
        assert_eq!(AccessibleState::new(1 << 10), None);
        let state = AccessibleState::new(AccessibleState::CHECKED | AccessibleState::FOCUSED)
            .expect("known bits");
        assert!(state.contains(AccessibleState::CHECKED));
        assert!(!state.contains(AccessibleState::BUSY));
        assert_eq!(AccessibleState::default().bits(), 0);
    }
}
