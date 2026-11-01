//! Why a request was refused. The client is told only the code (the wire's `ProtocolError`
//! carries nothing else); the compositor's log names the reason, so a client's mistake (a buffer
//! reused before its release, an extent off by a scale) is found from the log, not guessed.

use crate::presentation::Error as PresentError;
use oxyos_protocol::{ErrorCode, Format, Size};
use std::fmt;

/// Why a buffer request (or a frame naming a buffer) was refused with `InvalidBuffer`: by the
/// scene, or by the compositor's checks of a GPU buffer (docs/adr/0031-gpu-buffers.mdx). Only ids,
/// sizes, formats and indices: never anything else a request carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BufferRefusal {
    /// Id 0, which names no buffer.
    ZeroId,
    /// An `AttachBuffer` whose stride, offset and extent describe no buffer.
    BadLayout { buffer: u64 },
    /// An id the client already has a buffer by; a buffer destroyed but still held by a frame
    /// keeps its id until it is released.
    AlreadyAttached { buffer: u64, destroyed: bool },
    /// No buffer of the client's has this id (another client's buffers are not its).
    Unknown { buffer: u64 },
    /// The client destroyed it.
    Destroyed { buffer: u64 },
    /// A frame whose extent is not the extent the buffer was attached with.
    ExtentMismatch {
        buffer: u64,
        attached: Size,
        frame: Size,
    },
    /// Still being read: its release waits for the frame in flight to end.
    ReleaseDeferred { buffer: u64 },
    /// Held by another surface of the client (a buffer backs one surface at a time).
    AttachedElsewhere { buffer: u64 },
    /// Held by this surface: shown, presenting or queued, and not released yet.
    StillHeld { buffer: u64 },
    /// A frame of sequence 0 or of buffer 0.
    InvalidFrame,
    /// A frame whose extent is not the one the surface was configured with.
    SurfaceExtent { frame: Size, surface: Size },
    /// Any other refusal of the surface's presentation.
    Presentation(PresentError),
    /// A GPU buffer's plane, or a GPU buffer, while the compositor offers none.
    NoGpuBuffers,
    /// A plane whose descriptor is not a DMA-BUF.
    NotDmaBuf { buffer: u64 },
    /// A plane index past the most planes a buffer has.
    PlaneIndex { buffer: u64, plane: u8 },
    /// A plane attached twice.
    PlaneAttachedTwice { buffer: u64, plane: u8 },
    /// More buffers with planes attached and no `AttachDmabuf` yet than a client may have.
    TooManyPendingPlanes,
    /// An `AttachDmabuf` with no plane attached before it.
    NoPlanes { buffer: u64 },
    /// A format and modifier pair the compositor did not offer.
    FormatNotOffered {
        buffer: u64,
        format: Format,
        modifier: u64,
    },
    /// A plane count that is not the one its format and modifier have.
    PlaneCount { buffer: u64, named: u8, wanted: u32 },
    /// A plane the buffer names that was not attached.
    MissingPlane { buffer: u64, plane: u8 },
    /// A plane attached past the planes the buffer names.
    ExtraPlane { buffer: u64, plane: u8 },
    /// A linear plane whose offset and stride (4-byte aligned, a stride of at least a row) do not
    /// fit the buffer's rows in its DMA-BUF.
    PlaneOutside { buffer: u64, plane: u8 },
    /// Still held: a frame of it waits for its acquire point.
    AcquireWaiting { buffer: u64 },
    /// A frame with synchronization points whose buffer is not a GPU buffer.
    SyncWithoutGpuBuffer { buffer: u64 },
}

/// `width×height`.
struct Extent(Size);

impl fmt::Display for Extent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}×{}", self.0.width, self.0.height)
    }
}

impl fmt::Display for BufferRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::ZeroId => write!(f, "buffer id 0 names no buffer"),
            Self::BadLayout { buffer } => write!(
                f,
                "buffer {buffer}'s stride, offset and extent describe no buffer"
            ),
            Self::AlreadyAttached {
                buffer,
                destroyed: false,
            } => write!(f, "buffer {buffer} is already attached"),
            Self::AlreadyAttached {
                buffer,
                destroyed: true,
            } => write!(
                f,
                "buffer {buffer} is still attached: destroyed, but a frame holds it until it is released"
            ),
            Self::Unknown { buffer } => write!(f, "buffer {buffer} is not attached"),
            Self::Destroyed { buffer } => write!(f, "buffer {buffer} was destroyed"),
            Self::ExtentMismatch {
                buffer,
                attached,
                frame,
            } => write!(
                f,
                "the frame is {} but buffer {buffer} was attached at {}",
                Extent(frame),
                Extent(attached)
            ),
            Self::ReleaseDeferred { buffer } => write!(
                f,
                "buffer {buffer} is still held: its release waits for the frame in flight"
            ),
            Self::AttachedElsewhere { buffer } => {
                write!(f, "buffer {buffer} is held by another surface")
            }
            Self::StillHeld { buffer } => write!(
                f,
                "buffer {buffer} is still held by this surface (shown, presenting or queued) and was not released"
            ),
            Self::InvalidFrame => write!(f, "a frame of sequence 0 or of buffer 0"),
            Self::SurfaceExtent { frame, surface } => write!(
                f,
                "the frame is {} but the surface was configured at {}",
                Extent(frame),
                Extent(surface)
            ),
            Self::Presentation(error) => write!(f, "the surface's presentation refused it: {error:?}"),
            Self::NoGpuBuffers => write!(f, "the compositor offers no GPU buffers"),
            Self::NotDmaBuf { buffer } => {
                write!(f, "a plane of buffer {buffer} is not a DMA-BUF")
            }
            Self::PlaneIndex { buffer, plane } => {
                write!(f, "plane {plane} of buffer {buffer} is past the planes a buffer has")
            }
            Self::PlaneAttachedTwice { buffer, plane } => {
                write!(f, "plane {plane} of buffer {buffer} is already attached")
            }
            Self::TooManyPendingPlanes => write!(
                f,
                "too many buffers have planes attached and no AttachDmabuf"
            ),
            Self::NoPlanes { buffer } => write!(f, "buffer {buffer} has no plane attached"),
            Self::FormatNotOffered {
                buffer,
                format,
                modifier,
            } => write!(
                f,
                "buffer {buffer}'s format {format:?} with modifier {modifier:#x} is not offered"
            ),
            Self::PlaneCount {
                buffer,
                named,
                wanted,
            } => write!(
                f,
                "buffer {buffer} names {named} planes; its format and modifier have {wanted}"
            ),
            Self::MissingPlane { buffer, plane } => {
                write!(f, "plane {plane} of buffer {buffer} was not attached")
            }
            Self::ExtraPlane { buffer, plane } => write!(
                f,
                "plane {plane} of buffer {buffer} is past the planes the buffer names"
            ),
            Self::PlaneOutside { buffer, plane } => write!(
                f,
                "plane {plane} of buffer {buffer}: its offset and stride do not fit its rows in its DMA-BUF"
            ),
            Self::AcquireWaiting { buffer } => write!(
                f,
                "buffer {buffer} is still held: a frame of it waits for its acquire point"
            ),
            Self::SyncWithoutGpuBuffer { buffer } => write!(
                f,
                "the frame has synchronization points but buffer {buffer} is not a GPU buffer"
            ),
        }
    }
}

/// A refused request: the code the client is told and, where one is known, why.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Refusal {
    pub code: ErrorCode,
    pub reason: Option<BufferRefusal>,
}

impl Refusal {
    /// A buffer refused: `InvalidBuffer`, for `reason`.
    pub fn buffer(reason: BufferRefusal) -> Self {
        Self {
            code: ErrorCode::InvalidBuffer,
            reason: Some(reason),
        }
    }
}

impl From<ErrorCode> for Refusal {
    fn from(code: ErrorCode) -> Self {
        Self { code, reason: None }
    }
}

impl From<BufferRefusal> for Refusal {
    fn from(reason: BufferRefusal) -> Self {
        Self::buffer(reason)
    }
}

/// The code, and the reason in parentheses: `InvalidBuffer (buffer 7 is not attached)`.
impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.code)?;
        if let Some(reason) = self.reason {
            write!(f, " ({reason})")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refusal_reads_as_its_code_and_its_reason() {
        assert_eq!(
            Refusal::from(ErrorCode::UnknownSurface).to_string(),
            "UnknownSurface"
        );
        assert_eq!(
            Refusal::buffer(BufferRefusal::ReleaseDeferred { buffer: 7 }).to_string(),
            "InvalidBuffer (buffer 7 is still held: its release waits for the frame in flight)"
        );
        assert_eq!(
            Refusal::buffer(BufferRefusal::ExtentMismatch {
                buffer: 3,
                attached: Size::new(800, 600),
                frame: Size::new(1600, 1200),
            })
            .to_string(),
            "InvalidBuffer (the frame is 1600×1200 but buffer 3 was attached at 800×600)"
        );
        assert_eq!(
            BufferRefusal::FormatNotOffered {
                buffer: 2,
                format: Format::Argb8888,
                modifier: 0x0100_0000_0000_0001,
            }
            .to_string(),
            "buffer 2's format Argb8888 with modifier 0x100000000000001 is not offered"
        );
    }
}
