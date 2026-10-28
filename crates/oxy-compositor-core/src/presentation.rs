//! Per-surface presentation state: one queued replacement, one backend presentation in flight and
//! the last displayed content. The caller establishes GPU readiness before `begin_present` and
//! output completion before `complete_present`; returned metadata is not a GPU fence.

use oxy_protocol::Size;

pub type Extent = Size;

/// Metadata of a client buffer submission bound to a configure serial.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    pub sequence: u64,
    pub buffer: u64,
    pub configure: u64,
    pub extent: Extent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidExtent,
    InvalidFrame,
    StaleConfigure,
    WrongExtent,
    OutOfOrder,
    BufferBusy,
    PresentationBusy,
    NoPresentation,
    SerialExhausted,
}

#[derive(Debug)]
pub struct Presentation {
    serial: u64,
    extent: Extent,
    last_sequence: u64,
    displayed: Option<Frame>,
    queued: Option<Frame>,
    presenting: Option<Frame>,
}

impl Presentation {
    pub fn new(extent: Extent) -> Result<Self, Error> {
        if !extent.is_valid() {
            return Err(Error::InvalidExtent);
        }
        Ok(Self {
            serial: 1,
            extent,
            last_sequence: 0,
            displayed: None,
            queued: None,
            presenting: None,
        })
    }

    pub fn configuration(&self) -> (u64, Extent) {
        (self.serial, self.extent)
    }

    pub fn displayed(&self) -> Option<Frame> {
        self.displayed
    }

    /// Every buffer this surface still holds: displayed, in flight or queued.
    pub fn held_buffers(&self) -> impl Iterator<Item = Frame> + '_ {
        [self.displayed, self.presenting, self.queued]
            .into_iter()
            .flatten()
    }

    /// Drops all content (the surface is going away or was dismissed) and returns every frame
    /// still held, including one in flight, so the caller can retire their buffers once the
    /// backend no longer reads them. Serial and sequence ordering are kept.
    pub fn take_all(&mut self) -> Vec<Frame> {
        [
            self.displayed.take(),
            self.presenting.take(),
            self.queued.take(),
        ]
        .into_iter()
        .flatten()
        .collect()
    }

    pub fn is_presenting(&self) -> bool {
        self.presenting.is_some()
    }

    pub fn has_queued(&self) -> bool {
        self.queued.is_some()
    }

    pub fn configure(&mut self, extent: Extent) -> Result<Option<Frame>, Error> {
        if !extent.is_valid() {
            return Err(Error::InvalidExtent);
        }
        let serial = self.serial.checked_add(1).ok_or(Error::SerialExhausted)?;
        self.serial = serial;
        self.extent = extent;
        Ok(self.queued.take())
    }

    pub fn submit(&mut self, frame: Frame) -> Result<Option<Frame>, Error> {
        if frame.sequence == 0 || frame.buffer == 0 {
            return Err(Error::InvalidFrame);
        }
        if frame.configure != self.serial {
            return Err(Error::StaleConfigure);
        }
        if frame.extent != self.extent {
            return Err(Error::WrongExtent);
        }
        if frame.sequence <= self.last_sequence {
            return Err(Error::OutOfOrder);
        }
        if [self.displayed, self.presenting, self.queued]
            .into_iter()
            .flatten()
            .any(|held| held.buffer == frame.buffer)
        {
            return Err(Error::BufferBusy);
        }
        self.last_sequence = frame.sequence;
        Ok(self.queued.replace(frame))
    }

    pub fn begin_present(&mut self) -> Result<Option<Frame>, Error> {
        if self.presenting.is_some() {
            return Err(Error::PresentationBusy);
        }
        self.presenting = self.queued.take();
        Ok(self.presenting)
    }

    pub fn complete_present(&mut self) -> Result<Option<Frame>, Error> {
        let next = self.presenting.take().ok_or(Error::NoPresentation)?;
        Ok(self.displayed.replace(next))
    }

    pub fn retry_present(&mut self) -> Result<Option<Frame>, Error> {
        let frame = self.presenting.take().ok_or(Error::NoPresentation)?;
        if self.queued.is_some() || frame.configure != self.serial {
            return Ok(Some(frame));
        }
        self.queued = Some(frame);
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: Extent = Extent {
        width: 800,
        height: 600,
    };
    fn frame(sequence: u64, buffer: u64) -> Frame {
        Frame {
            sequence,
            buffer,
            configure: 1,
            extent: SIZE,
        }
    }
    fn show(surface: &mut Presentation, next: Frame) {
        surface.submit(next).unwrap();
        assert_eq!(surface.begin_present().unwrap(), Some(next));
        surface.complete_present().unwrap();
    }

    #[test]
    fn retains_last_content_until_backend_completion() {
        let mut surface = Presentation::new(SIZE).unwrap();
        assert_eq!(surface.displayed(), None);
        surface.submit(frame(1, 10)).unwrap();
        assert_eq!(surface.displayed(), None);
        surface.begin_present().unwrap();
        assert_eq!(surface.displayed(), None);
        assert_eq!(surface.complete_present().unwrap(), None);
        surface.submit(frame(2, 11)).unwrap();
        surface.begin_present().unwrap();
        assert_eq!(surface.displayed(), Some(frame(1, 10)));
        assert_eq!(surface.complete_present().unwrap(), Some(frame(1, 10)));
        assert_eq!(surface.displayed(), Some(frame(2, 11)));
    }

    #[test]
    fn latest_queued_frame_wins_without_releasing_in_flight_content() {
        let mut surface = Presentation::new(SIZE).unwrap();
        show(&mut surface, frame(1, 10));
        surface.submit(frame(2, 11)).unwrap();
        surface.begin_present().unwrap();
        surface.submit(frame(3, 12)).unwrap();
        assert_eq!(surface.submit(frame(4, 13)).unwrap(), Some(frame(3, 12)));
        assert_eq!(surface.begin_present(), Err(Error::PresentationBusy));
        assert_eq!(surface.complete_present().unwrap(), Some(frame(1, 10)));
        assert_eq!(surface.begin_present().unwrap(), Some(frame(4, 13)));
    }

    #[test]
    fn rejects_busy_buffers_stale_sizes_and_old_sequences_without_mutation() {
        let mut surface = Presentation::new(SIZE).unwrap();
        show(&mut surface, frame(1, 10));
        assert_eq!(surface.submit(frame(2, 10)), Err(Error::BufferBusy));
        assert_eq!(surface.submit(frame(1, 11)), Err(Error::OutOfOrder));
        assert_eq!(
            surface.submit(Frame {
                extent: Extent {
                    width: 1,
                    height: 1
                },
                ..frame(2, 11)
            }),
            Err(Error::WrongExtent)
        );
        assert_eq!(
            surface.submit(Frame {
                configure: 99,
                ..frame(2, 11)
            }),
            Err(Error::StaleConfigure)
        );
        surface.submit(frame(2, 11)).unwrap();
        assert_eq!(surface.submit(frame(3, 11)), Err(Error::BufferBusy));
        surface.begin_present().unwrap();
        assert_eq!(surface.submit(frame(3, 11)), Err(Error::BufferBusy));
        surface.complete_present().unwrap();
        assert_eq!(surface.submit(frame(3, 10)).unwrap(), None);
    }

    #[test]
    fn resize_discards_only_queued_frames_and_preserves_visible_content() {
        let mut surface = Presentation::new(SIZE).unwrap();
        show(&mut surface, frame(1, 10));
        surface.submit(frame(2, 11)).unwrap();
        let next_size = Extent {
            width: 1024,
            height: 768,
        };
        assert_eq!(surface.configure(next_size).unwrap(), Some(frame(2, 11)));
        assert_eq!(surface.displayed(), Some(frame(1, 10)));
        assert_eq!(surface.submit(frame(3, 12)), Err(Error::StaleConfigure));
        let next = Frame {
            configure: 2,
            extent: next_size,
            ..frame(3, 12)
        };
        show(&mut surface, next);
        assert_eq!(surface.displayed(), Some(next));
    }

    #[test]
    fn failed_present_can_retry_or_yield_to_newer_content() {
        let mut surface = Presentation::new(SIZE).unwrap();
        show(&mut surface, frame(1, 10));
        surface.submit(frame(2, 11)).unwrap();
        surface.begin_present().unwrap();
        assert_eq!(surface.retry_present().unwrap(), None);
        assert_eq!(surface.displayed(), Some(frame(1, 10)));
        surface.begin_present().unwrap();
        surface.submit(frame(3, 12)).unwrap();
        assert_eq!(surface.retry_present().unwrap(), Some(frame(2, 11)));
        assert_eq!(surface.begin_present().unwrap(), Some(frame(3, 12)));
    }

    #[test]
    fn resize_during_present_does_not_release_in_flight_buffer() {
        let mut surface = Presentation::new(SIZE).unwrap();
        surface.submit(frame(1, 10)).unwrap();
        surface.begin_present().unwrap();
        assert_eq!(surface.configure(SIZE).unwrap(), None);
        assert_eq!(surface.retry_present().unwrap(), Some(frame(1, 10)));
        assert_eq!(surface.begin_present().unwrap(), None);
        assert_eq!(surface.complete_present(), Err(Error::NoPresentation));
    }

    #[test]
    fn invalid_configuration_is_atomic_and_surfaces_are_isolated() {
        let mut first = Presentation::new(SIZE).unwrap();
        let second = Presentation::new(SIZE).unwrap();
        assert_eq!(
            first.configure(Extent {
                width: 0,
                height: 600
            }),
            Err(Error::InvalidExtent)
        );
        assert_eq!(first.configuration(), (1, SIZE));
        show(&mut first, frame(1, 1));
        assert_eq!(second.displayed(), None);
        first.serial = u64::MAX;
        assert_eq!(first.configure(SIZE), Err(Error::SerialExhausted));
        assert_eq!(first.displayed(), Some(frame(1, 1)));
    }
}
