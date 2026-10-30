use super::*;
use rustix::fs::MemfdFlags;

fn memfd(bytes: &[u8]) -> OwnedFd {
    let fd = rustix::fs::memfd_create("test", MemfdFlags::CLOEXEC).unwrap();
    rustix::io::write(&fd, bytes).unwrap();
    fd
}

/// A DMA-BUF of shared memory holding `bytes`, or None (with why) where the kernel has no udmabuf.
fn dmabuf(bytes: &[u8]) -> Option<DmaBuf> {
    match shared_dmabuf(bytes.len() as u64) {
        Ok((memfd, buffer)) => {
            rustix::io::pwrite(&memfd, bytes, 0).unwrap();
            Some(buffer)
        }
        Err(error) => {
            eprintln!("no udmabuf here ({error}), skipped; tools/gpu-vm runs it");
            None
        }
    }
}

fn node() -> Option<Arc<RenderNode>> {
    let node = RenderNode::with_timelines(None).map(Arc::new);
    if node.is_none() {
        eprintln!("no render node with timeline syncobjs here, skipped; tools/gpu-vm runs it");
    }
    node
}

#[test]
fn a_memfd_or_a_pipe_is_not_a_dmabuf() {
    assert_eq!(
        DmaBuf::from_fd(memfd(&[0; 4096])).unwrap_err(),
        Error::NotDmaBuf
    );
    let (read, _write) = rustix::pipe::pipe().unwrap();
    assert_eq!(DmaBuf::from_fd(read).unwrap_err(), Error::NotDmaBuf);
}

#[test]
fn a_linear_plane_fits_within_its_buffer_with_a_stride_of_at_least_a_row() {
    // 100 rows of 400 bytes, 512 apart, from 1024: the last ends at 1024 + 512 * 99 + 400.
    let end = 1024 + 512 * 99 + 400;
    assert!(linear_plane_fits(end, 1024, 512, 400, 100));
    assert!(!linear_plane_fits(end - 1, 1024, 512, 400, 100));
    assert!(!linear_plane_fits(u64::MAX, 0, 396, 400, 100), "stride under a row");
    assert!(!linear_plane_fits(u64::MAX, 2, 512, 400, 100), "offset unaligned");
    assert!(!linear_plane_fits(u64::MAX, 0, 514, 400, 100), "stride unaligned");
    assert!(!linear_plane_fits(u64::MAX, 0, 512, 400, 0), "no rows");
    assert!(!linear_plane_fits(1 << 40, u32::MAX - 3, u32::MAX - 3, 400, u32::MAX));
}

#[test]
fn a_mapped_dmabuf_reads_the_rows_asked_for_and_nothing_beyond_it() {
    let bytes: Vec<u8> = (0..8192u32).map(|i| (i % 251) as u8).collect();
    let Some(buffer) = dmabuf(&bytes) else {
        return;
    };
    assert_eq!(buffer.size(), 8192);
    let mapping = Mapping::new(buffer).unwrap();
    // Rows 2 and 3 of 16 bytes, 64 apart, from 128.
    let mut out = vec![0; 32];
    mapping.read_rows(128, 64, 16, 2..4, &mut out).unwrap();
    assert_eq!(&out[..16], &bytes[256..272]);
    assert_eq!(&out[16..], &bytes[320..336]);
    let mut out = vec![0; 16];
    assert!(mapping.read_rows(8192 - 8, 64, 16, 0..1, &mut out).is_err());
    assert!(mapping.read_rows(0, 64, 16, 0..2, &mut out).is_err(), "out too short");
}

#[test]
fn a_timeline_point_signals_its_eventfd_and_the_latest_signalled_is_known() {
    let Some(node) = node() else {
        return;
    };
    let timeline = Timeline::create(&node).unwrap();
    assert_eq!(timeline.signalled().unwrap(), 0);
    let two = timeline.eventfd(2, false).unwrap();
    timeline.signal(1).unwrap();
    assert!(!sync_file_signalled(two.as_fd()), "point 1 is not point 2");
    timeline.signal(2).unwrap();
    assert!(sync_file_signalled(two.as_fd()));
    assert_eq!(timeline.signalled().unwrap(), 2);
    // A point already signalled is readable at once.
    assert!(sync_file_signalled(timeline.eventfd(1, false).unwrap().as_fd()));
}

#[test]
fn a_timeline_exported_and_imported_is_one_timeline() {
    let Some(node) = node() else {
        return;
    };
    let other = Arc::new(RenderNode::with_timelines(Some(node.dev())).unwrap());
    let ours = Timeline::create(&node).unwrap();
    let theirs = Timeline::import(&other, ours.export().unwrap().as_fd()).unwrap();
    ours.signal(7).unwrap();
    assert_eq!(theirs.signalled().unwrap(), 7);
    // Not a syncobj: refused by the kernel.
    assert!(Timeline::import(&other, memfd(&[0; 16]).as_fd()).is_err());
}

#[test]
fn sync_files_go_into_points_and_points_out_as_sync_files() {
    let Some(node) = node() else {
        return;
    };
    let timeline = Timeline::create(&node).unwrap();
    assert!(
        timeline.export_sync_file(1).is_err(),
        "a point no one gave a fence has no sync file"
    );
    timeline.signal(1).unwrap();
    let fence = timeline.export_sync_file(1).unwrap();
    assert!(sync_file_signalled(fence.as_fd()));
    let other = Timeline::create(&node).unwrap();
    other.import_sync_file(5, fence.as_fd()).unwrap();
    assert_eq!(other.signalled().unwrap(), 5);
    let merged = merge_sync_files(fence.as_fd(), other.export_sync_file(5).unwrap().as_fd()).unwrap();
    assert!(sync_file_signalled(merged.as_fd()));
}
