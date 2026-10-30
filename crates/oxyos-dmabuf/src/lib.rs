//! DMA-BUFs and DRM syncobj timelines, for applications' GPU buffers
//! (docs/adr/0031-gpu-buffers.mdx).
//!
//! - [`DmaBuf`]: a descriptor checked to be a DMA-BUF, with its size: what the compositor accepts
//!   as a plane (decision 11) and what a client attaches.
//! - [`Mapping`]: a linear DMA-BUF mapped read-only, read row by row within
//!   `DMA_BUF_IOCTL_SYNC`'s bracket: the CPU compositor's reads (decision 10).
//! - [`RenderNode`] and [`Timeline`]: DRM syncobj timelines (decision 6), their points signalled,
//!   watched with an eventfd, and bridged to and from sync files.
//! - [`udmabuf`]: a DMA-BUF made of sealed shared memory, for clients that draw on the CPU into a
//!   GPU buffer and for tests.
//!
//! The workspace forbids `unsafe`; this crate denies it and allows it only on the mapping and on
//! the ioctls `drm` does not wrap, each with the contract it relies on.

use drm::control::{syncobj, Device as _};
use rustix::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd};
use std::io;
use std::path::Path;
use std::sync::Arc;

/// The linear modifier (`DRM_FORMAT_MOD_LINEAR`): rows one after another, readable by any device.
pub const MODIFIER_LINEAR: u64 = 0;
/// `DRM_FORMAT_MOD_INVALID`: no modifier, never accepted.
pub const MODIFIER_INVALID: u64 = 0x00ff_ffff_ffff_ffff;

/// Why a descriptor is not taken as a DMA-BUF.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// Another kind of file (a memfd, a pipe...).
    NotDmaBuf,
    /// The kernel did not answer about it.
    Io,
}

/// A DMA-BUF: a descriptor the kernel describes as one (its `fdinfo` names the exporter), and its
/// size, which cannot change.
#[derive(Debug)]
pub struct DmaBuf {
    fd: OwnedFd,
    size: u64,
}

impl DmaBuf {
    /// Takes `fd` only if it is a DMA-BUF. The kernel's `fdinfo` of a DMA-BUF has its exporter
    /// (`exp_name`) and its size (Documentation/filesystems/proc.rst); no other file has an
    /// exporter.
    pub fn from_fd(fd: OwnedFd) -> Result<Self, Error> {
        let info = std::fs::read_to_string(format!("/proc/self/fdinfo/{}", fd.as_raw_fd()))
            .map_err(|_| Error::Io)?;
        if !info.lines().any(|line| line.starts_with("exp_name:")) {
            return Err(Error::NotDmaBuf);
        }
        let size = rustix::fs::seek(&fd, rustix::fs::SeekFrom::End(0)).map_err(|_| Error::Io)?;
        rustix::fs::seek(&fd, rustix::fs::SeekFrom::Start(0)).map_err(|_| Error::Io)?;
        Ok(Self { fd, size })
    }

    /// Its size in bytes.
    pub fn size(&self) -> u64 {
        self.size
    }

    pub fn try_clone(&self) -> io::Result<Self> {
        Ok(Self {
            fd: self.fd.try_clone()?,
            size: self.size,
        })
    }

    pub fn into_fd(self) -> OwnedFd {
        self.fd
    }
}

impl AsFd for DmaBuf {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}

/// Whether a linear plane of `rows` rows of `row_bytes`, `stride` apart from `offset`, lies within
/// a DMA-BUF of `size` bytes: its stride at least a row, both aligned to 4.
pub fn linear_plane_fits(size: u64, offset: u32, stride: u32, row_bytes: u64, rows: u32) -> bool {
    if rows == 0 || u64::from(stride) < row_bytes || stride % 4 != 0 || offset % 4 != 0 {
        return false;
    }
    u64::from(stride)
        .checked_mul(u64::from(rows) - 1)
        .and_then(|before| before.checked_add(u64::from(offset)))
        .and_then(|before| before.checked_add(row_bytes))
        .is_some_and(|end| end <= size)
}

// ---- ioctls `drm` does not wrap ---------------------------------------------------------------

/// `DMA_BUF_IOCTL_SYNC`'s flags (linux/dma-buf.h).
const SYNC_READ: u64 = 1;
const SYNC_START: u64 = 0;
const SYNC_END: u64 = 4;

#[repr(C)]
struct DmaBufSync {
    flags: u64,
}

/// `DMA_BUF_IOCTL_SYNC` (`_IOW('b', 0, struct dma_buf_sync)`): the start or end of a CPU access.
#[allow(unsafe_code)]
fn dma_buf_sync(fd: BorrowedFd<'_>, flags: u64) -> io::Result<()> {
    use rustix::ioctl::{ioctl, opcode, Setter};
    const SYNC: rustix::ioctl::Opcode = opcode::write::<DmaBufSync>(b'b', 0);
    // SAFETY: SYNC is DMA_BUF_IOCTL_SYNC, which reads one `struct dma_buf_sync` (a u64 of flags,
    // as `DmaBufSync` is laid out) and writes nothing back.
    unsafe { ioctl(fd, Setter::<SYNC, DmaBufSync>::new(DmaBufSync { flags })) }?;
    Ok(())
}

#[repr(C)]
struct SyncMergeData {
    name: [u8; 32],
    fd2: i32,
    fence: i32,
    flags: u32,
    pad: u32,
}

/// Merges two sync files into a new one that signals once both have (`SYNC_IOC_MERGE`).
#[allow(unsafe_code)]
pub fn merge_sync_files(a: BorrowedFd<'_>, b: BorrowedFd<'_>) -> io::Result<OwnedFd> {
    use rustix::fd::FromRawFd;
    use rustix::ioctl::{ioctl, opcode, Updater};
    const MERGE: rustix::ioctl::Opcode = opcode::read_write::<SyncMergeData>(b'>', 3);
    let mut data = SyncMergeData {
        name: *b"oxyos-release\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0",
        fd2: b.as_raw_fd(),
        fence: -1,
        flags: 0,
        pad: 0,
    };
    // SAFETY: MERGE is SYNC_IOC_MERGE, which reads and writes one `struct sync_merge_data`
    // (`SyncMergeData` has its layout); `fd2` is a live descriptor for the call's duration.
    unsafe { ioctl(a, Updater::<MERGE, SyncMergeData>::new(&mut data)) }?;
    if data.fence < 0 {
        return Err(io::Error::other("SYNC_IOC_MERGE gave no fence"));
    }
    // SAFETY: on success the kernel installed `fence` as a new descriptor this process owns.
    Ok(unsafe { OwnedFd::from_raw_fd(data.fence) })
}

/// Whether a sync file has signalled (it polls readable once it has).
pub fn sync_file_signalled(fd: BorrowedFd<'_>) -> bool {
    use rustix::event::{poll, PollFd, PollFlags, Timespec};
    let mut fds = [PollFd::new(&fd, PollFlags::IN)];
    let now = Timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    poll(&mut fds, Some(&now)).is_ok_and(|ready| ready == 1)
}

#[repr(C)]
struct SyncobjHandle {
    handle: u32,
    flags: u32,
    fd: i32,
    pad: u32,
}

/// `DRM_IOCTL_SYNCOBJ_FD_TO_HANDLE` with `DRM_SYNCOBJ_FD_TO_HANDLE_FLAGS_IMPORT_SYNC_FILE`: the
/// binary syncobj `handle` takes the sync file's fence (`drm` names no handle for it).
#[allow(unsafe_code)]
fn import_sync_file(
    node: &RenderNode,
    handle: syncobj::Handle,
    fence: BorrowedFd<'_>,
) -> io::Result<()> {
    use rustix::ioctl::{ioctl, opcode, Updater};
    const FD_TO_HANDLE: rustix::ioctl::Opcode = opcode::read_write::<SyncobjHandle>(b'd', 0xC2);
    let mut args = SyncobjHandle {
        handle: u32::from(handle),
        flags: 1,
        fd: fence.as_raw_fd(),
        pad: 0,
    };
    // SAFETY: FD_TO_HANDLE is DRM_IOCTL_SYNCOBJ_FD_TO_HANDLE, which reads and writes one
    // `struct drm_syncobj_handle` (`SyncobjHandle` has its layout); `fence` is live for the call.
    unsafe { ioctl(node, Updater::<FD_TO_HANDLE, SyncobjHandle>::new(&mut args)) }?;
    Ok(())
}

#[repr(C)]
struct UdmabufCreate {
    memfd: u32,
    flags: u32,
    offset: u64,
    size: u64,
}

/// `UDMABUF_CREATE`, whose answer is the new DMA-BUF's descriptor.
struct CreateUdmabuf(UdmabufCreate);

// SAFETY: the opcode is UDMABUF_CREATE (`_IOW('u', 0x42, struct udmabuf_create)`), which reads one
// `struct udmabuf_create` (`UdmabufCreate` has its layout) and returns a new descriptor.
#[allow(unsafe_code)]
unsafe impl rustix::ioctl::Ioctl for CreateUdmabuf {
    type Output = OwnedFd;
    const IS_MUTATING: bool = false;

    fn opcode(&self) -> rustix::ioctl::Opcode {
        rustix::ioctl::opcode::write::<UdmabufCreate>(b'u', 0x42)
    }

    fn as_ptr(&mut self) -> *mut std::ffi::c_void {
        (&mut self.0 as *mut UdmabufCreate).cast()
    }

    unsafe fn output_from_ptr(
        out: rustix::ioctl::IoctlOutput,
        _: *mut std::ffi::c_void,
    ) -> rustix::io::Result<OwnedFd> {
        use rustix::fd::FromRawFd;
        // SAFETY: a successful UDMABUF_CREATE returns a new descriptor this process owns.
        Ok(unsafe { OwnedFd::from_raw_fd(out) })
    }
}

/// A DMA-BUF of `size` bytes of `memfd` (sealed against shrinking, its size a multiple of the
/// page size), made by `/dev/udmabuf`: what a client drawing on the CPU hands over as a GPU
/// buffer, and what the tests import.
#[allow(unsafe_code)]
pub fn udmabuf(memfd: BorrowedFd<'_>, size: u64) -> io::Result<DmaBuf> {
    let device = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/udmabuf")?;
    let create = CreateUdmabuf(UdmabufCreate {
        memfd: u32::try_from(memfd.as_raw_fd()).map_err(|_| io::Error::other("bad descriptor"))?,
        // UDMABUF_FLAGS_CLOEXEC.
        flags: 1,
        offset: 0,
        size,
    });
    // SAFETY: `CreateUdmabuf` describes UDMABUF_CREATE exactly (above); `memfd` is live.
    let fd = unsafe { rustix::ioctl::ioctl(&device, create) }?;
    DmaBuf::from_fd(fd).map_err(|_| io::Error::other("udmabuf made no DMA-BUF"))
}

/// Sealed shared memory of `size` bytes (rounded up to pages) as a DMA-BUF, through `udmabuf`; the
/// memfd is returned too, for the caller to write into.
pub fn shared_dmabuf(size: u64) -> io::Result<(OwnedFd, DmaBuf)> {
    use rustix::fs::{MemfdFlags, SealFlags};
    let page = 4096;
    let size = size.div_ceil(page) * page;
    let memfd =
        rustix::fs::memfd_create("oxyos-dmabuf", MemfdFlags::CLOEXEC | MemfdFlags::ALLOW_SEALING)?;
    rustix::fs::ftruncate(&memfd, size)?;
    rustix::fs::fcntl_add_seals(&memfd, SealFlags::SHRINK | SealFlags::GROW | SealFlags::SEAL)?;
    let buffer = udmabuf(memfd.as_fd(), size)?;
    Ok((memfd, buffer))
}

// ---- mapping -------------------------------------------------------------------------------

/// A DMA-BUF mapped read-only, whole. Its size cannot change, so the mapping stays valid: no
/// client can make a reader fault by shrinking it, as it could a memfd it did not seal.
pub struct Mapping {
    buffer: DmaBuf,
    address: std::ptr::NonNull<u8>,
}

impl Mapping {
    #[allow(unsafe_code)]
    pub fn new(buffer: DmaBuf) -> io::Result<Self> {
        use rustix::mm::{mmap, MapFlags, ProtFlags};
        let len = usize::try_from(buffer.size).map_err(|_| io::Error::other("too large"))?;
        if len == 0 {
            return Err(io::Error::other("empty DMA-BUF"));
        }
        // SAFETY: a new shared read-only mapping of the whole DMA-BUF at an address the kernel
        // chooses; it aliases no Rust object, and it is unmapped only in `Drop`.
        let address = unsafe {
            mmap(
                std::ptr::null_mut(),
                len,
                ProtFlags::READ,
                MapFlags::SHARED,
                &buffer.fd,
                0,
            )
        }?;
        let address = std::ptr::NonNull::new(address.cast()).ok_or(io::ErrorKind::Other)?;
        Ok(Self { buffer, address })
    }

    pub fn buffer(&self) -> &DmaBuf {
        &self.buffer
    }

    /// Copies rows `rows` of a plane (`row_bytes` each, `stride` apart from `offset`) into `out`,
    /// one after another, within the CPU access bracket (`DMA_BUF_IOCTL_SYNC` start and end of a
    /// read), which also waits for writes the buffer's exporter still has in flight.
    #[allow(unsafe_code)]
    pub fn read_rows(
        &self,
        offset: u32,
        stride: u32,
        row_bytes: usize,
        rows: std::ops::Range<u32>,
        out: &mut [u8],
    ) -> io::Result<()> {
        let count = rows.end.saturating_sub(rows.start) as usize;
        if out.len() != count * row_bytes {
            return Err(io::Error::other("rows do not fit the output"));
        }
        if count == 0 {
            return Ok(());
        }
        let last = u64::from(offset)
            + u64::from(stride) * u64::from(rows.end - 1)
            + row_bytes as u64;
        if last > self.buffer.size || (row_bytes as u64) > u64::from(stride) {
            return Err(io::Error::other("rows beyond the DMA-BUF"));
        }
        dma_buf_sync(self.buffer.as_fd(), SYNC_START | SYNC_READ)?;
        for (index, row) in rows.enumerate() {
            let start = u64::from(offset) + u64::from(stride) * u64::from(row);
            // SAFETY: `start + row_bytes` is within the mapping (checked above against its size);
            // the destination is `out`'s own row; source and destination never overlap. The
            // source is plain bytes another process may write meanwhile: every byte value is a
            // valid u8, and such a write only tears its own content.
            unsafe {
                std::ptr::copy_nonoverlapping(
                    self.address.as_ptr().add(start as usize),
                    out.as_mut_ptr().add(index * row_bytes),
                    row_bytes,
                );
            }
        }
        dma_buf_sync(self.buffer.as_fd(), SYNC_END | SYNC_READ)
    }
}

#[allow(unsafe_code)]
impl Drop for Mapping {
    fn drop(&mut self) {
        // SAFETY: the mapping made in `new`, of the buffer's size; nothing refers to it after.
        let _ = unsafe { rustix::mm::munmap(self.address.as_ptr().cast(), self.buffer.size as usize) };
    }
}

impl std::fmt::Debug for Mapping {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Mapping").field("size", &self.buffer.size).finish()
    }
}

// ---- timelines -------------------------------------------------------------------------------

/// A DRM render node, opened for its syncobjs.
#[derive(Debug)]
pub struct RenderNode {
    file: std::fs::File,
    dev: u64,
}

impl AsFd for RenderNode {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.file.as_fd()
    }
}

impl drm::Device for RenderNode {}
impl drm::control::Device for RenderNode {}

impl RenderNode {
    pub fn open(path: &Path) -> io::Result<Self> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)?;
        let stat = rustix::fs::fstat(&file)?;
        Ok(Self {
            file,
            dev: stat.st_rdev,
        })
    }

    /// Its device number (`st_rdev`), as `GpuDevice` names it.
    pub fn dev(&self) -> u64 {
        self.dev
    }

    /// Whether it has timeline syncobjs (`DRM_CAP_SYNCOBJ_TIMELINE`).
    pub fn has_timelines(&self) -> bool {
        use drm::Device as _;
        self.get_driver_capability(drm::DriverCapability::TimelineSyncObj)
            .is_ok_and(|value| value == 1)
    }

    /// The machine's render nodes, in order (`/dev/dri/renderD*`).
    pub fn paths() -> Vec<std::path::PathBuf> {
        let mut paths: Vec<_> = std::fs::read_dir("/dev/dri")
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("renderD"))
            })
            .collect();
        paths.sort();
        paths
    }

    /// The render node of device `dev` if it has timelines, else the first that has them.
    pub fn with_timelines(dev: Option<u64>) -> Option<Self> {
        let nodes: Vec<Self> = Self::paths()
            .iter()
            .filter_map(|path| Self::open(path).ok())
            .filter(Self::has_timelines)
            .collect();
        let preferred = dev.and_then(|dev| nodes.iter().position(|node| node.dev == dev));
        nodes.into_iter().nth(preferred.unwrap_or(0))
    }
}

/// A DRM syncobj timeline: points that signal in order, each once (docs/adr/0031, decision 6).
#[derive(Debug)]
pub struct Timeline {
    node: Arc<RenderNode>,
    handle: syncobj::Handle,
}

impl Timeline {
    pub fn create(node: &Arc<RenderNode>) -> io::Result<Self> {
        Ok(Self {
            handle: node.create_syncobj(false)?,
            node: node.clone(),
        })
    }

    /// The syncobj `fd` names (another process's export, [`Timeline::export`]); anything else is
    /// refused by the kernel.
    pub fn import(node: &Arc<RenderNode>, fd: BorrowedFd<'_>) -> io::Result<Self> {
        Ok(Self {
            handle: node.fd_to_syncobj(fd, false)?,
            node: node.clone(),
        })
    }

    /// A descriptor naming it, for another process (`ImportTimeline`) or a Vulkan timeline
    /// semaphore (`OPAQUE_FD`).
    pub fn export(&self) -> io::Result<OwnedFd> {
        self.node.syncobj_to_fd(self.handle, false)
    }

    pub fn signal(&self, point: u64) -> io::Result<()> {
        self.node
            .syncobj_timeline_signal(&[self.handle], &[point])
    }

    /// The latest point signalled.
    pub fn signalled(&self) -> io::Result<u64> {
        let mut points = [0];
        self.node
            .syncobj_timeline_query(&[self.handle], &mut points, false)?;
        Ok(points[0])
    }

    /// An eventfd (non-blocking, close-on-exec) the kernel makes readable once `point` signals,
    /// or, `available`, once a fence for it exists.
    pub fn eventfd(&self, point: u64, available: bool) -> io::Result<OwnedFd> {
        use rustix::event::{eventfd, EventfdFlags};
        let fd = eventfd(0, EventfdFlags::CLOEXEC | EventfdFlags::NONBLOCK)?;
        self.node
            .syncobj_eventfd(self.handle, point, fd.as_fd(), available)?;
        Ok(fd)
    }

    /// `point` signals when the sync file `fence` does.
    pub fn import_sync_file(&self, point: u64, fence: BorrowedFd<'_>) -> io::Result<()> {
        let binary = self.node.create_syncobj(false)?;
        let result = import_sync_file(&self.node, binary, fence).and_then(|()| {
            self.node
                .syncobj_timeline_transfer(binary, self.handle, 0, point)
        });
        let _ = self.node.destroy_syncobj(binary);
        result
    }

    /// The fence of `point` as a sync file. The point must have one (it was signalled, or given
    /// a fence by whoever signals it); otherwise the kernel refuses.
    pub fn export_sync_file(&self, point: u64) -> io::Result<OwnedFd> {
        let binary = self.node.create_syncobj(false)?;
        let result = self
            .node
            .syncobj_timeline_transfer(self.handle, binary, point, 0)
            .and_then(|()| self.node.syncobj_to_fd(binary, true));
        let _ = self.node.destroy_syncobj(binary);
        result
    }
}

impl Drop for Timeline {
    fn drop(&mut self) {
        let _ = self.node.destroy_syncobj(self.handle);
    }
}

#[cfg(test)]
mod tests;
