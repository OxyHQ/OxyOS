//! The compositor's turn above every application (docs/adr/0066-quality-of-service.mdx,
//! decision 2).
//!
//! The session's turn thread reads input, serves clients, composes and flips. Every application
//! runs at nice 0 on the same scheduler, so under load the turn waits behind their threads. Two
//! requests put it ahead, neither of them real time (a thread that composes and waits on GPU
//! fences must never be demoted or killed by rtkit's `RLIMIT_RTTIME` watchdog, which is what made
//! Mutter move its KMS thread back from real time to high priority):
//!
//! - **rtkit's high priority** ([`ask_rtkit`]): `MakeThreadHighPriorityWithPID` asks
//!   `org.freedesktop.RealtimeKit1` on the system bus for nice −15 ([`TURN_NICE`], clamped to its
//!   `MinNiceLevel`), which polkit gives a process of an active or inactive local session (a seat's)
//!   and refuses any other. rtkit also marks the thread `SCHED_RESET_ON_FORK`: the threads and
//!   processes it starts begin at nice 0.
//! - **A short slice** ([`request_slice`]): EEVDF (the fair scheduler since Linux 6.6) takes a
//!   thread's requested runtime on `SCHED_OTHER` (`sched_setattr`'s `sched_runtime`, honoured since
//!   6.12) as its slice: a shorter request is an earlier virtual deadline, so the woken turn
//!   preempts batch work at its default slice instead of waiting for it to run out. It needs no
//!   privilege. The thread's policy, nice and flags are kept, and `SCHED_RESET_ON_FORK` is set, so
//!   what it starts gets the default slice.
//!
//! [`raise_calling_thread`] makes both for the calling thread from a thread of its own (rtkit may
//! take a moment to answer, and the turn's first frame never waits on it) and says what it got.
//! The development compositor asks a played rtkit ([`played`], [`Rtkit::played_from_env`]), never
//! the system's: its runs are in no seat session, and the system's would refuse them.

use std::io;
use std::time::Duration;

pub mod played;
#[cfg(test)]
mod tests;

/// The nice the turn thread asks rtkit for: its default `MinNiceLevel`, PipeWire's main thread's
/// neighbourhood (−11), above every application (0).
pub const TURN_NICE: i32 = -15;
/// The slice the turn thread and each Bloom host's UI and JavaScript threads request: a millisecond, a
/// third of the default (`sysctl_sched_base_slice` scaled by the CPUs, 2.8 ms on the ThinkPad).
pub const SLICE: Duration = Duration::from_millis(1);
/// rtkit's name, object and interface.
pub const RTKIT: &str = "org.freedesktop.RealtimeKit1";
pub const RTKIT_PATH: &str = "/org/freedesktop/RealtimeKit1";
/// Where the development compositor finds the played rtkit's bus.
pub const PLAYED_BUS_VARIABLE: &str = "OXYOS_RTKIT_BUS";

/// `SCHED_FLAG_RESET_ON_FORK` (`linux/sched.h`).
const RESET_ON_FORK: u64 = 0x01;
/// `SCHED_ATTR_SIZE_VER0`: the fields up to `sched_period`.
const SCHED_ATTR_SIZE: u32 = 48;

/// `struct sched_attr` as far as `SCHED_ATTR_SIZE_VER0` (`linux/sched/types.h`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct SchedAttr {
    size: u32,
    policy: u32,
    flags: u64,
    nice: i32,
    priority: u32,
    runtime: u64,
    deadline: u64,
    period: u64,
}

/// A thread's scheduling as the kernel reports it (`sched_getattr`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scheduling {
    /// `SCHED_OTHER` (0), `SCHED_BATCH` (3), `SCHED_IDLE` (5), or a real-time one.
    pub policy: u32,
    pub nice: i32,
    /// Its slice, in nanoseconds: the one it requested, else the default (a fair policy's alone).
    pub slice_ns: u64,
    /// What it starts begins at the default policy, nice 0 if its own is below, and the default
    /// slice.
    pub reset_on_fork: bool,
}

/// The calling thread's id.
pub fn tid() -> i32 {
    #[allow(unsafe_code)]
    // SAFETY: gettid takes nothing and touches no memory.
    unsafe {
        libc::gettid()
    }
}

fn get_attributes(tid: i32) -> io::Result<SchedAttr> {
    let mut attributes = SchedAttr::default();
    #[allow(unsafe_code)]
    // SAFETY: `attributes` is a live, writable `sched_attr` of the size given for the call's
    // duration; the kernel writes at most that many bytes into it.
    let result = unsafe {
        libc::syscall(
            libc::SYS_sched_getattr,
            tid,
            &mut attributes as *mut SchedAttr,
            SCHED_ATTR_SIZE,
            0,
        )
    };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(attributes)
}

/// The scheduling of the thread `tid` (0: the calling thread).
pub fn scheduling(tid: i32) -> io::Result<Scheduling> {
    let attributes = get_attributes(tid)?;
    Ok(Scheduling {
        policy: attributes.policy,
        nice: attributes.nice,
        slice_ns: attributes.runtime,
        reset_on_fork: attributes.flags & RESET_ON_FORK != 0,
    })
}

/// Whether `policy` is the fair scheduler's: `SCHED_OTHER`, `SCHED_BATCH` or `SCHED_IDLE`.
fn fair(policy: u32) -> bool {
    matches!(
        policy as i32,
        libc::SCHED_OTHER | libc::SCHED_BATCH | libc::SCHED_IDLE
    )
}

/// Requests `slice` for the thread `tid` (0: the calling thread), of this process or another of
/// the same user's: its policy, nice and flags kept (a nice rtkit gave stays), and
/// `SCHED_RESET_ON_FORK` set, so the threads and processes it starts get the default slice. The
/// kernel clamps a request to 0.1 to 100 ms. Only a fair policy has a slice.
pub fn request_slice(tid: i32, slice: Duration) -> io::Result<()> {
    let current = get_attributes(tid)?;
    if !fair(current.policy) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("policy {} has no slice", current.policy),
        ));
    }
    let attributes = SchedAttr {
        size: SCHED_ATTR_SIZE,
        policy: current.policy,
        flags: (current.flags & RESET_ON_FORK) | RESET_ON_FORK,
        nice: current.nice,
        priority: 0,
        runtime: u64::try_from(slice.as_nanos()).unwrap_or(u64::MAX),
        deadline: 0,
        period: 0,
    };
    #[allow(unsafe_code)]
    // SAFETY: `attributes` is a live, initialised `sched_attr` of the size it says for the call's
    // duration; the kernel only reads it.
    let result = unsafe {
        libc::syscall(
            libc::SYS_sched_setattr,
            tid,
            &attributes as *const SchedAttr,
            0,
        )
    };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Why rtkit gave nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// No rtkit on that bus.
    Absent,
    /// rtkit (polkit, or its own limits) refused: a process in no seat session, for one.
    Refused(String),
    /// The call failed otherwise.
    Failed(String),
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::Absent => write!(f, "no rtkit"),
            Refusal::Refused(why) => write!(f, "rtkit refused: {why}"),
            Refusal::Failed(why) => write!(f, "rtkit failed: {why}"),
        }
    }
}

fn refusal(error: zbus::Error) -> Refusal {
    let name = match &error {
        zbus::Error::MethodError(name, _, _) => Some(name.as_str().to_string()),
        zbus::Error::FDO(error) => match **error {
            zbus::fdo::Error::ServiceUnknown(_)
            | zbus::fdo::Error::NameHasNoOwner(_)
            | zbus::fdo::Error::UnknownObject(_) => return Refusal::Absent,
            zbus::fdo::Error::AccessDenied(_) => {
                return Refusal::Refused(error.to_string());
            }
            _ => None,
        },
        _ => None,
    };
    match name.as_deref() {
        Some(
            "org.freedesktop.DBus.Error.ServiceUnknown"
            | "org.freedesktop.DBus.Error.NameHasNoOwner"
            | "org.freedesktop.DBus.Error.UnknownObject",
        ) => Refusal::Absent,
        Some(name)
            if name == "org.freedesktop.DBus.Error.AccessDenied"
                || name.ends_with(".NotAuthorized")
                || name.ends_with(".PermissionDenied") =>
        {
            Refusal::Refused(error.to_string())
        }
        _ => Refusal::Failed(error.to_string()),
    }
}

/// Asks rtkit on `bus` for nice `nice` on the thread `tid` of this process, as low as its
/// `MinNiceLevel` allows: the nice given.
pub fn ask_rtkit(bus: &zbus::blocking::Connection, tid: i32, nice: i32) -> Result<i32, Refusal> {
    let proxy = zbus::blocking::proxy::Builder::<zbus::blocking::Proxy<'_>>::new(bus)
        .destination(RTKIT)
        .and_then(|builder| builder.path(RTKIT_PATH))
        .and_then(|builder| builder.interface(RTKIT))
        .map(|builder| builder.cache_properties(zbus::proxy::CacheProperties::No))
        .and_then(|builder| builder.build())
        .map_err(refusal)?;
    let minimum: i32 = proxy.get_property("MinNiceLevel").map_err(refusal)?;
    let nice = nice.max(minimum);
    proxy
        .call_method(
            "MakeThreadHighPriorityWithPID",
            &(u64::from(std::process::id()), tid as u64, nice),
        )
        .map_err(refusal)?;
    Ok(nice)
}

/// Where rtkit is asked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rtkit {
    /// The system's, on the system bus: a session's.
    System,
    /// A played one on the bus at this address: the development compositor's runs.
    At(String),
    /// Nowhere: the slice alone.
    Nowhere,
}

impl Rtkit {
    /// The development compositor's: the played rtkit on the bus `OXYOS_RTKIT_BUS` names, else
    /// none (never the system's, whose polkit refuses a process in no seat session, and which on a
    /// developer's own seat would give a fixture what no scenario expects).
    pub fn played_from_env() -> Self {
        match std::env::var(PLAYED_BUS_VARIABLE) {
            Ok(address) if !address.is_empty() => Rtkit::At(address),
            _ => Rtkit::Nowhere,
        }
    }

    fn connect(&self) -> Result<Option<zbus::blocking::Connection>, Refusal> {
        match self {
            Rtkit::System => oxyos_seat::system_bus()
                .map(Some)
                .map_err(|error| Refusal::Failed(format!("the system bus: {error}"))),
            Rtkit::At(address) => zbus::blocking::connection::Builder::address(address.as_str())
                .and_then(|builder| builder.build())
                .map(Some)
                .map_err(|error| Refusal::Failed(format!("the bus at {address}: {error}"))),
            Rtkit::Nowhere => Ok(None),
        }
    }
}

/// What a thread was given.
#[derive(Debug)]
pub struct Raised {
    pub tid: i32,
    /// The nice rtkit gave, or why none; `None` where rtkit was not asked.
    pub nice: Option<Result<i32, Refusal>>,
    /// Whether the slice was set.
    pub slice: io::Result<()>,
    /// The thread's scheduling afterwards.
    pub scheduling: io::Result<Scheduling>,
}

impl Raised {
    /// One line for the log.
    pub fn describe(&self) -> String {
        let rtkit = match &self.nice {
            Some(Ok(nice)) => format!("nice {nice} from rtkit"),
            Some(Err(refusal)) => format!("{refusal}"),
            None => "rtkit not asked".to_string(),
        };
        let slice = match &self.slice {
            Ok(()) => "a slice requested".to_string(),
            Err(error) => format!("no slice ({error})"),
        };
        let now = match &self.scheduling {
            Ok(s) => format!(
                "now nice {}, a {:.2} ms slice{}",
                s.nice,
                s.slice_ns as f64 / 1e6,
                if s.reset_on_fork {
                    ", reset on fork"
                } else {
                    ""
                }
            ),
            Err(error) => format!("its scheduling unread ({error})"),
        };
        format!("thread {}: {rtkit}, {slice}; {now}", self.tid)
    }
}

/// Raises the thread `tid` of this process: rtkit asked for `nice` (where `rtkit` says), then
/// `slice` requested, in that order (rtkit sets the thread's policy again, which a slice requested
/// first might not survive). Waits for rtkit's answer: run it off the thread raised.
pub fn raise(tid: i32, rtkit: &Rtkit, nice: i32, slice: Duration) -> Raised {
    let nice = match rtkit.connect() {
        Ok(Some(bus)) => Some(ask_rtkit(&bus, tid, nice)),
        Ok(None) => None,
        Err(refusal) => Some(Err(refusal)),
    };
    let slice = request_slice(tid, slice);
    Raised {
        tid,
        nice,
        slice,
        scheduling: scheduling(tid),
    }
}

/// Raises the calling thread (the turn) from a thread of its own, which logs what it got as
/// `[label] the turn (…)` and ends: the caller goes on at once.
pub fn raise_calling_thread(
    rtkit: Rtkit,
    nice: i32,
    slice: Duration,
    label: &'static str,
) -> io::Result<std::thread::JoinHandle<Raised>> {
    let turn = tid();
    std::thread::Builder::new()
        .name("oxyos-priority".into())
        .spawn(move || {
            let raised = raise(turn, &rtkit, nice, slice);
            eprintln!("[{label}] the turn's priority: {}", raised.describe());
            raised
        })
}
