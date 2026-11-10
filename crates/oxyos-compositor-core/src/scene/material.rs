//! A surface's material regions (docs/adr/0069-the-platforms-material.mdx): the rounded
//! rectangles, in buffer pixels, under which the compositor blurs what it has composed beneath the
//! surface before drawing it, as a Mac's vibrancy. A client tells them in order before the Submit
//! they belong to (`MaterialRegion`: `index` 0 begins the list, `last` ends it); the list is the
//! frame's that Submit gives, and holds from when that frame is displayed until a later list's
//! frame is. The scene keeps them; the program running the compositor reads them for what it
//! draws (`material_regions`).

use super::{Scene, SurfaceKey};
use oxyos_protocol::{ErrorCode, Rect, MAX_MATERIAL_REGIONS};

/// A material region: a rectangle in the surface's buffer pixels and its corners' radius.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MaterialRegion {
    pub rect: Rect,
    pub radius: u32,
}

/// The lists a surface keeps: the one displayed and those of frames submitted after it, at most.
const MAX_LISTS: usize = 4;

/// A surface's material: the list being told, the one told whole that the next Submit takes, and
/// the lists of frames submitted, by their sequence, oldest first.
#[derive(Debug, Default)]
pub(super) struct Material {
    receiving: Vec<MaterialRegion>,
    next: Option<Vec<MaterialRegion>>,
    lists: Vec<(u64, Vec<MaterialRegion>)>,
}

impl Material {
    /// The list a frame of `sequence` (the one displayed) draws with.
    fn for_frame(&self, sequence: u64) -> &[MaterialRegion] {
        self.lists
            .iter()
            .rev()
            .find(|(of, _)| *of <= sequence)
            .map_or(&[], |(_, list)| list.as_slice())
    }

    /// The Submit of frame `sequence`, `displayed` the one on screen: the list told whole is its,
    /// and a list no frame displayed or to come can draw with goes.
    pub(super) fn submitted(&mut self, sequence: u64, displayed: Option<u64>) {
        if let Some(list) = self.next.take() {
            self.lists.push((sequence, list));
        }
        if let Some(shown) = displayed {
            if let Some(current) = self.lists.iter().rposition(|(of, _)| *of <= shown) {
                self.lists.drain(..current);
            }
        }
        // Frames are presented in order and at most a few wait (Presentation): past that, the
        // oldest list after the one displayed goes, its frame superseded before it was shown.
        while self.lists.len() > MAX_LISTS {
            self.lists.remove(1);
        }
    }
}

impl Scene {
    /// One region of a surface's material list (`MaterialRegion`).
    pub(super) fn material_region(
        &mut self,
        key: SurfaceKey,
        index: u8,
        last: bool,
        rect: Rect,
        radius: u32,
    ) -> Result<(), ErrorCode> {
        let node = self.nodes.get_mut(&key).ok_or(ErrorCode::UnknownSurface)?;
        let material = &mut node.material;
        if index == 0 {
            material.receiving.clear();
        } else if usize::from(index) != material.receiving.len() {
            material.receiving.clear();
            return Err(ErrorCode::InvalidOrder);
        }
        let empty = rect.width == 0 || rect.height == 0;
        if empty {
            // None: the one region of a list, its first and last.
            if index != 0 || !last || radius != 0 {
                return Err(ErrorCode::InvalidGeometry);
            }
            material.next = Some(Vec::new());
            return Ok(());
        }
        if material.receiving.len() >= MAX_MATERIAL_REGIONS {
            material.receiving.clear();
            return Err(ErrorCode::QuotaExceeded);
        }
        if rect.x < 0
            || rect.y < 0
            || radius > rect.width.min(rect.height) / 2
            || i64::from(rect.x) + i64::from(rect.width) > i64::from(oxyos_protocol::MAX_DIMENSION)
            || i64::from(rect.y) + i64::from(rect.height) > i64::from(oxyos_protocol::MAX_DIMENSION)
        {
            material.receiving.clear();
            return Err(ErrorCode::InvalidGeometry);
        }
        material.receiving.push(MaterialRegion { rect, radius });
        if last {
            material.next = Some(std::mem::take(&mut material.receiving));
        }
        Ok(())
    }

    /// The material regions of the frame `key` displays, in its buffer pixels: none for a surface
    /// unknown, without a frame displayed or that never told any. What lies outside the frame's
    /// extent is the compositor's to clip.
    pub fn material_regions(&self, key: SurfaceKey) -> &[MaterialRegion] {
        let Some(node) = self.nodes.get(&key) else {
            return &[];
        };
        match node.presentation.displayed() {
            Some(frame) => node.material.for_frame(frame.sequence),
            None => &[],
        }
    }
}
