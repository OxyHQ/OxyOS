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
    /// What happened to a submitted frame. Released is a separate event: a presented frame keeps
    /// its buffer until replaced and until every consumer has finished.
    Outcome: u8 { Presented = 1, Discarded = 2 }
}
wire_enum! {
    /// Source of a feedback timestamp. Synthetic timestamps never describe physical display time.
    Clock: u8 { Synthetic = 1, Monotonic = 2 }
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
