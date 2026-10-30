//! Fixed-width little-endian field types. Every decoder validates its value range, so a decoded
//! message holds only representable values; semantic checks that need compositor state live in
//! the compositor.

use crate::{invalid, MAX_COORDINATE, MAX_DIMENSION};
use std::io;

pub(crate) trait Wire: Sized {
    const SIZE: usize;
    fn put(&self, out: &mut Vec<u8>);
    fn get(bytes: &[u8]) -> io::Result<Self>;
}

macro_rules! integer {
    ($($type:ty),*) => {$(
        impl Wire for $type {
            const SIZE: usize = std::mem::size_of::<$type>();
            fn put(&self, out: &mut Vec<u8>) {
                out.extend(self.to_le_bytes());
            }
            fn get(bytes: &[u8]) -> io::Result<Self> {
                Ok(<$type>::from_le_bytes(bytes.try_into().expect("schema-checked length")))
            }
        }
    )*};
}
integer!(u8, u16, u32, u64, i32);

impl Wire for bool {
    const SIZE: usize = 1;
    fn put(&self, out: &mut Vec<u8>) {
        out.push(u8::from(*self));
    }
    fn get(bytes: &[u8]) -> io::Result<Self> {
        match bytes[0] {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(invalid("invalid boolean")),
        }
    }
}

/// Wire enums: a closed set of u8 or u16 values; anything else is an invalid message.
macro_rules! wire_enum {
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

        impl Wire for $name {
            const SIZE: usize = std::mem::size_of::<$repr>();
            fn put(&self, out: &mut Vec<u8>) {
                (*self as $repr).put(out);
            }
            fn get(bytes: &[u8]) -> io::Result<Self> {
                Self::from_raw(<$repr>::get(bytes)?)
                    .ok_or_else(|| invalid(concat!("invalid ", stringify!($name))))
            }
        }
    };
}

wire_enum! {
    /// Point of the anchor rectangle the popup is attached to.
    Anchor: u8 { Center = 0, Top = 1, Bottom = 2, Left = 3, Right = 4, TopLeft = 5, BottomLeft = 6, TopRight = 7, BottomRight = 8 }
}
wire_enum! {
    /// Direction in which the popup extends from the anchor point.
    Gravity: u8 { Center = 0, Top = 1, Bottom = 2, Left = 3, Right = 4, TopLeft = 5, BottomLeft = 6, TopRight = 7, BottomRight = 8 }
}
wire_enum! {
    /// Pixel formats as DRM fourcc codes, so shared-memory and future DMA-BUF buffers agree.
    Format: u32 { Xrgb8888 = 0x3432_5258, Argb8888 = 0x3432_5241 }
}
wire_enum! {
    /// Privileged shell roles. Each requires a session grant; the label alone authorizes nothing.
    ShellKind: u8 { Background = 1, Panel = 2, Overlay = 3, Notification = 4 }
}
wire_enum! {
    PanelEdge: u8 { None = 0, Top = 1, Bottom = 2, Left = 3, Right = 4 }
}
wire_enum! {
    DismissReason: u8 {
        OutsidePress = 1, ParentGone = 2, ParentHidden = 3, GrabDenied = 4, Locked = 5,
        OutputRemoved = 6, Unplaceable = 7, AuthorizationRevoked = 8, Superseded = 9,
    }
}
wire_enum! {
    PointerKind: u8 { Enter = 1, Leave = 2, Motion = 3, Press = 4, Release = 5, Cancel = 6 }
}
wire_enum! {
    TouchKind: u8 { Down = 1, Motion = 2, Up = 3, Cancel = 4 }
}
wire_enum! {
    /// Where a touchpad gesture is: begun, going on, ended, or cancelled (ended without effect).
    GesturePhase: u8 { Begin = 1, Update = 2, End = 3, Cancel = 4 }
}
wire_enum! {
    /// What happened to a submitted frame. Released is a separate event: a presented frame keeps
    /// its buffer until replaced and until every consumer has finished.
    Outcome: u8 { Presented = 1, Discarded = 2 }
}
wire_enum! {
    /// What came of a lock surface's password (ADR 0010): it unlocked the session, it was wrong,
    /// another is being checked, or attempts must wait (`wait_s` says how long).
    AuthOutcome: u8 { Accepted = 1, Rejected = 2, Busy = 3, Throttled = 4 }
}
wire_enum! {
    /// Source of a feedback timestamp. Synthetic timestamps never describe physical display time.
    Clock: u8 { Synthetic = 1, Monotonic = 2 }
}
wire_enum! {
    /// What an accessible node is, for an assistive technology (ADR 0003). Unknown roles are
    /// published as `Generic`, never guessed.
    AccessibleRole: u8 {
        Generic = 0, Button = 1, Link = 2, Text = 3, Heading = 4, Image = 5, TextInput = 6,
        Menu = 7, MenuItem = 8, CheckBox = 9, Radio = 10, Switch = 11, Tab = 12, TabList = 13,
        List = 14, ListItem = 15, Dialog = 16, Alert = 17, ProgressBar = 18, Slider = 19,
        Search = 20, Toolbar = 21, ScrollView = 22, Group = 23, ComboBox = 24, Summary = 25,
        Grid = 26,
    }
}
wire_enum! {
    /// Which text of an accessible node a chunk belongs to: its name, its value, or the labels
    /// of its custom actions (one per line, in order: `Custom` action `n` is line `n`).
    AccessibleField: u8 { Name = 1, Value = 2, Actions = 3 }
}
wire_enum! {
    /// What an assistive technology asks an application to do with a node, as Android's
    /// accessibility actions: activate (a double tap) and focus any node; increment, decrement,
    /// scroll forward and backward, dismiss and set the selection a node that offers them
    /// (`AccessibleActions`); a custom action by its index among the node's.
    AccessibleAction: u8 {
        Activate = 1, Focus = 2, Increment = 3, Decrement = 4, ScrollForward = 5,
        ScrollBackward = 6, Dismiss = 7, Custom = 8, SetSelection = 9,
    }
}
wire_enum! {
    /// How an announcement interrupts an assistive technology: after what it is saying
    /// (`Polite`), or at once (`Assertive`), as Android's live regions.
    Politeness: u8 { Polite = 1, Assertive = 2 }
}
wire_enum! {
    /// A screen reader's command from its keys (ADR 0014): move its cursor to the next or the
    /// previous element, or activate the element under it.
    ReaderCommand: u8 { Next = 1, Previous = 2, Activate = 3 }
}
wire_enum! {
    /// What a text field is for (ADR 0008).
    TextPurpose: u8 {
        Normal = 0, Digits = 1, Number = 2, Phone = 3, Url = 4, Email = 5, Name = 6,
        Password = 7, Pin = 8, Date = 9, Time = 10,
    }
}
wire_enum! {
    /// The pointer's shape a surface wants while the pointer is over it, as Android's pointer
    /// icons name them: the arrow, text, a hand on what can be pressed, and the rest.
    PointerShape: u8 {
        Arrow = 1, Text = 2, Hand = 3, NotAllowed = 4, Wait = 5, Help = 6, Crosshair = 7,
        Move = 8, Grab = 9, Grabbing = 10, ResizeHorizontal = 11, ResizeVertical = 12,
        ResizeTopLeft = 13, ResizeTopRight = 14, ZoomIn = 15, ZoomOut = 16,
    }
}
wire_enum! {
    /// How a segment of a composition is drawn: the whole composition, the segment being
    /// converted, or text the input method marks as wrong.
    PreeditStyle: u8 { Composing = 1, Converting = 2, Error = 3 }
}
wire_enum! {
    ErrorCode: u16 {
        InvalidMessage = 1, InvalidOrder = 2, UnsupportedVersion = 3, Unauthorized = 4,
        UnknownSurface = 5, SurfaceExists = 6, InvalidParent = 7, InvalidPositioner = 8,
        InvalidGeometry = 9, QuotaExceeded = 10, InvalidRole = 11, InvalidBuffer = 12,
    }
}

/// Logical rectangle. A zero width or height is only meaningful where a field documents it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }

    pub fn right(self) -> i64 {
        i64::from(self.x) + i64::from(self.width)
    }

    pub fn bottom(self) -> i64 {
        i64::from(self.y) + i64::from(self.height)
    }

    pub fn contains_point(self, x: i32, y: i32) -> bool {
        x >= self.x && i64::from(x) < self.right() && y >= self.y && i64::from(y) < self.bottom()
    }

    pub fn contains(self, other: Rect) -> bool {
        other.x >= self.x
            && other.y >= self.y
            && other.right() <= self.right()
            && other.bottom() <= self.bottom()
    }

    pub fn intersects(self, other: Rect) -> bool {
        !self.is_empty()
            && !other.is_empty()
            && i64::from(self.x) < other.right()
            && i64::from(other.x) < self.right()
            && i64::from(self.y) < other.bottom()
            && i64::from(other.y) < self.bottom()
    }

    pub fn offset(self, dx: i32, dy: i32) -> Option<Rect> {
        Some(Rect {
            x: self.x.checked_add(dx)?,
            y: self.y.checked_add(dy)?,
            ..self
        })
    }
}

impl Wire for Rect {
    const SIZE: usize = 16;
    fn put(&self, out: &mut Vec<u8>) {
        self.x.put(out);
        self.y.put(out);
        self.width.put(out);
        self.height.put(out);
    }
    fn get(bytes: &[u8]) -> io::Result<Self> {
        let rect = Rect {
            x: i32::get(&bytes[0..4])?,
            y: i32::get(&bytes[4..8])?,
            width: u32::get(&bytes[8..12])?,
            height: u32::get(&bytes[12..16])?,
        };
        let coordinate = -MAX_COORDINATE..=MAX_COORDINATE;
        if !coordinate.contains(&rect.x)
            || !coordinate.contains(&rect.y)
            || rect.width > MAX_DIMENSION
            || rect.height > MAX_DIMENSION
        {
            return Err(invalid("rectangle out of range"));
        }
        Ok(rect)
    }
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

impl Wire for AccessibleState {
    const SIZE: usize = 2;
    fn put(&self, out: &mut Vec<u8>) {
        self.0.put(out);
    }
    fn get(bytes: &[u8]) -> io::Result<Self> {
        Self::new(u16::get(bytes)?).ok_or_else(|| invalid("unknown accessible state"))
    }
}

/// The actions a node offers beyond activate and focus, as bits (Android's
/// `AccessibilityNodeInfo` actions). Unknown bits are invalid.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct AccessibleActions(u16);

impl AccessibleActions {
    /// An adjustable node's value up or down.
    pub const INCREMENT: u16 = 1 << 0;
    pub const DECREMENT: u16 = 1 << 1;
    /// A scrollable node, by a page.
    pub const SCROLL_FORWARD: u16 = 1 << 2;
    pub const SCROLL_BACKWARD: u16 = 1 << 3;
    /// What closes the node's dialog or popup (Android's back on it).
    pub const DISMISS: u16 = 1 << 4;
    /// A text field's selection, in bytes of its value.
    pub const SET_SELECTION: u16 = 1 << 5;
    const ALL: u16 = (1 << 6) - 1;

    /// The actions with these bits, if every bit is known.
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

impl Wire for AccessibleActions {
    const SIZE: usize = 2;
    fn put(&self, out: &mut Vec<u8>) {
        self.0.put(out);
    }
    fn get(bytes: &[u8]) -> io::Result<Self> {
        Self::new(u16::get(bytes)?).ok_or_else(|| invalid("unknown accessible actions"))
    }
}

/// A text field's hints, as bits (ADR 0008). Unknown bits are invalid.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TextHints(u32);

impl TextHints {
    pub const NO_PREDICTION: u32 = 1 << 0;
    pub const NO_CAPITALIZATION: u32 = 1 << 1;
    /// Content an input method must not see, whatever the field's purpose.
    pub const SENSITIVE: u32 = 1 << 2;
    pub const MULTILINE: u32 = 1 << 3;
    const ALL: u32 = (1 << 4) - 1;

    /// The hints with these bits, if every bit is known.
    pub const fn new(bits: u32) -> Option<Self> {
        if bits & !Self::ALL == 0 {
            Some(Self(bits))
        } else {
            None
        }
    }

    pub const fn bits(self) -> u32 {
        self.0
    }

    pub const fn contains(self, bit: u32) -> bool {
        self.0 & bit == bit
    }
}

impl Wire for TextHints {
    const SIZE: usize = 4;
    fn put(&self, out: &mut Vec<u8>) {
        self.0.put(out);
    }
    fn get(bytes: &[u8]) -> io::Result<Self> {
        Self::new(u32::get(bytes)?).ok_or_else(|| invalid("unknown text hints"))
    }
}

/// Bytes of UTF-8 one `Text` event carries.
pub const TEXT_CHUNK_BYTES: usize = 32;

/// Committed text: 1 to 32 bytes of UTF-8 that never split a character. Longer text is sent as
/// consecutive chunks.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextChunk {
    length: u8,
    bytes: [u8; TEXT_CHUNK_BYTES],
}

impl TextChunk {
    /// The chunk holding `text`, if it is 1 to 32 bytes long.
    pub fn new(text: &str) -> Option<Self> {
        if text.is_empty() || text.len() > TEXT_CHUNK_BYTES {
            return None;
        }
        let mut bytes = [0; TEXT_CHUNK_BYTES];
        bytes[..text.len()].copy_from_slice(text.as_bytes());
        Some(Self {
            length: text.len() as u8,
            bytes,
        })
    }

    /// Splits `text` into chunks at character boundaries.
    pub fn split(text: &str) -> Vec<Self> {
        let mut chunks = Vec::new();
        let mut rest = text;
        while !rest.is_empty() {
            let mut end = rest.len().min(TEXT_CHUNK_BYTES);
            while !rest.is_char_boundary(end) {
                end -= 1;
            }
            chunks.push(Self::new(&rest[..end]).expect("1 to 32 bytes"));
            rest = &rest[end..];
        }
        chunks
    }

    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..usize::from(self.length)]).expect("validated UTF-8")
    }
}

impl std::fmt::Debug for TextChunk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.as_str().fmt(f)
    }
}

impl Wire for TextChunk {
    const SIZE: usize = 1 + TEXT_CHUNK_BYTES;
    fn put(&self, out: &mut Vec<u8>) {
        out.push(self.length);
        out.extend(self.bytes);
    }
    fn get(bytes: &[u8]) -> io::Result<Self> {
        let length = usize::from(bytes[0]);
        if !(1..=TEXT_CHUNK_BYTES).contains(&length) {
            return Err(invalid("text length out of range"));
        }
        let text = &bytes[1..=length];
        // Unused bytes are zero, so every chunk has one encoding.
        if bytes[1 + length..].iter().any(|&b| b != 0) {
            return Err(invalid("text padding is not zero"));
        }
        let text = std::str::from_utf8(text).map_err(|_| invalid("text is not UTF-8"))?;
        Ok(Self::new(text).expect("checked length"))
    }
}

/// Part of a password from a lock surface (ADR 0010): a [`TextChunk`] that is never printed.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct SecretChunk(TextChunk);

impl SecretChunk {
    /// The chunk holding `text`, if it is 1 to 32 bytes long.
    pub fn new(text: &str) -> Option<Self> {
        TextChunk::new(text).map(Self)
    }

    /// Splits `secret` into chunks at character boundaries.
    pub fn split(secret: &str) -> Vec<Self> {
        TextChunk::split(secret).into_iter().map(Self).collect()
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl std::fmt::Debug for SecretChunk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretChunk(…)")
    }
}

impl Wire for SecretChunk {
    const SIZE: usize = TextChunk::SIZE;
    fn put(&self, out: &mut Vec<u8>) {
        self.0.put(out);
    }
    fn get(bytes: &[u8]) -> io::Result<Self> {
        TextChunk::get(bytes).map(Self)
    }
}

/// Composition (preedit) text from an input method: like [`TextChunk`] but possibly empty, which
/// ends the composition without committing anything.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PreeditText(Option<TextChunk>);

impl PreeditText {
    /// The preedit holding `text` (empty clears it), if it fits in one chunk.
    pub fn new(text: &str) -> Option<Self> {
        if text.is_empty() {
            Some(Self(None))
        } else {
            TextChunk::new(text).map(|chunk| Self(Some(chunk)))
        }
    }

    pub fn as_str(&self) -> &str {
        self.0.as_ref().map_or("", TextChunk::as_str)
    }
}

impl Wire for PreeditText {
    const SIZE: usize = TextChunk::SIZE;
    fn put(&self, out: &mut Vec<u8>) {
        match &self.0 {
            Some(chunk) => chunk.put(out),
            None => out.extend([0; TextChunk::SIZE]),
        }
    }
    fn get(bytes: &[u8]) -> io::Result<Self> {
        if bytes[0] == 0 {
            if bytes.iter().any(|&b| b != 0) {
                return Err(invalid("text padding is not zero"));
            }
            return Ok(Self(None));
        }
        TextChunk::get(bytes).map(|chunk| Self(Some(chunk)))
    }
}

/// Logical or physical size; both dimensions within 1..=MAX_DIMENSION.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

impl Size {
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    pub fn is_valid(self) -> bool {
        (1..=MAX_DIMENSION).contains(&self.width) && (1..=MAX_DIMENSION).contains(&self.height)
    }
}

impl Wire for Size {
    const SIZE: usize = 8;
    fn put(&self, out: &mut Vec<u8>) {
        self.width.put(out);
        self.height.put(out);
    }
    fn get(bytes: &[u8]) -> io::Result<Self> {
        let size = Size {
            width: u32::get(&bytes[0..4])?,
            height: u32::get(&bytes[4..8])?,
        };
        if size.is_valid() {
            Ok(size)
        } else {
            Err(invalid("size out of range"))
        }
    }
}

/// Constraint adjustments the compositor may apply when a popup would leave its usable area.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Adjust(u8);

impl Adjust {
    pub const NONE: Adjust = Adjust(0);
    pub const SLIDE_X: Adjust = Adjust(1);
    pub const SLIDE_Y: Adjust = Adjust(2);
    pub const FLIP_X: Adjust = Adjust(4);
    pub const FLIP_Y: Adjust = Adjust(8);
    pub const RESIZE_X: Adjust = Adjust(16);
    pub const RESIZE_Y: Adjust = Adjust(32);
    /// Mirror the horizontal components of anchor, gravity and offset (right-to-left layout).
    pub const RTL: Adjust = Adjust(64);
    pub const ALL: Adjust = Adjust(127);

    pub const fn union(self, other: Adjust) -> Adjust {
        Adjust(self.0 | other.0)
    }

    pub const fn has(self, other: Adjust) -> bool {
        self.0 & other.0 == other.0
    }

    /// The flags with these bits, if every bit is a known flag.
    pub const fn from_bits(bits: u8) -> Option<Adjust> {
        if bits & !Adjust::ALL.0 == 0 {
            Some(Adjust(bits))
        } else {
            None
        }
    }
}

impl Wire for Adjust {
    const SIZE: usize = 1;
    fn put(&self, out: &mut Vec<u8>) {
        out.push(self.0);
    }
    fn get(bytes: &[u8]) -> io::Result<Self> {
        Adjust::from_bits(bytes[0]).ok_or_else(|| invalid("unknown adjustment flags"))
    }
}

/// Popup placement request, relative to the parent surface's logical origin.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Positioner {
    /// Anchor rectangle in parent-local coordinates; must have a positive size.
    pub anchor_rect: Rect,
    pub size: Size,
    pub anchor: Anchor,
    pub gravity: Gravity,
    pub adjust: Adjust,
    pub offset_x: i32,
    pub offset_y: i32,
}

impl Wire for Positioner {
    const SIZE: usize = Rect::SIZE + Size::SIZE + 3 + 8;
    fn put(&self, out: &mut Vec<u8>) {
        self.anchor_rect.put(out);
        self.size.put(out);
        self.anchor.put(out);
        self.gravity.put(out);
        self.adjust.put(out);
        self.offset_x.put(out);
        self.offset_y.put(out);
    }
    fn get(bytes: &[u8]) -> io::Result<Self> {
        let offsets = -MAX_COORDINATE..=MAX_COORDINATE;
        let positioner = Positioner {
            anchor_rect: Rect::get(&bytes[0..16])?,
            size: Size::get(&bytes[16..24])?,
            anchor: Anchor::get(&bytes[24..25])?,
            gravity: Gravity::get(&bytes[25..26])?,
            adjust: Adjust::get(&bytes[26..27])?,
            offset_x: i32::get(&bytes[27..31])?,
            offset_y: i32::get(&bytes[31..35])?,
        };
        if positioner.anchor_rect.is_empty()
            || !offsets.contains(&positioner.offset_x)
            || !offsets.contains(&positioner.offset_y)
        {
            return Err(invalid("invalid positioner"));
        }
        Ok(positioner)
    }
}

/// Largest accepted buffer, in bytes, including any offset.
pub const MAX_BUFFER_BYTES: u64 = 256 << 20;

impl Format {
    pub const fn bytes_per_pixel(self) -> u32 {
        4
    }
}

/// Layout of a client buffer inside its shared memory object.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BufferLayout {
    pub format: Format,
    pub extent: Size,
    pub stride: u32,
    pub offset: u32,
}

impl BufferLayout {
    /// Bytes the memory object must contain, or None if the layout is invalid: stride shorter
    /// than a row, misaligned stride/offset or more than `MAX_BUFFER_BYTES`.
    pub fn required_bytes(&self) -> Option<u64> {
        let row = u64::from(self.extent.width) * u64::from(self.format.bytes_per_pixel());
        let stride = u64::from(self.stride);
        if !self.extent.is_valid() || stride < row || stride % 4 != 0 || self.offset % 4 != 0 {
            return None;
        }
        let total = u64::from(self.offset)
            .checked_add(stride.checked_mul(u64::from(self.extent.height) - 1)?)?
            .checked_add(row)?;
        (total <= MAX_BUFFER_BYTES).then_some(total)
    }
}
