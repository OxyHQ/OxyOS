//! The scheduler's idle classes for the indexer (docs/adr/0051-search.mdx, decision 4): the CPU's
//! (`SCHED_IDLE`) and the disk's (`IOPRIO_CLASS_IDLE`), for the calling thread alone, which the
//! threads and processes it starts inherit. A thread of the idle class runs only on a CPU nothing
//! else wants (cgroup v2's `cpu.idle`, `CPUWeight=idle`, gives a cgroup's tasks this same class),
//! and its reads wait for a disk nothing else is reading: indexing never takes time from what the
//! person is doing, and a laptop's cores sleep as soon as the person's work lets them.
//!
//! rustix has neither call, so they are made through libc.

/// `IOPRIO_WHO_PROCESS` (`linux/ioprio.h`): `who` 0 is the calling thread.
const IOPRIO_WHO_PROCESS: libc::c_int = 1;
/// `IOPRIO_CLASS_IDLE`, in the value's top bits (`IOPRIO_CLASS_SHIFT` 13).
const IOPRIO_IDLE: libc::c_int = 3 << 13;

/// Puts the calling thread in the CPU's and the disk's idle classes; a refusal is said and
/// changes nothing else.
pub fn idle_thread() {
    let parameters = libc::sched_param { sched_priority: 0 };
    #[allow(unsafe_code)]
    // SAFETY: `parameters` is a live, initialised `sched_param` for the call's duration; pid 0 is
    // the calling thread, whose policy alone changes.
    let scheduled = unsafe { libc::sched_setscheduler(0, libc::SCHED_IDLE, &parameters) };
    if scheduled != 0 {
        eprintln!(
            "[search] the indexer keeps its priority: {}",
            std::io::Error::last_os_error()
        );
    }
    let prioritised = {
        #[allow(unsafe_code)]
        // SAFETY: `ioprio_set` takes three integers and touches no memory of the caller's.
        unsafe {
            libc::syscall(libc::SYS_ioprio_set, IOPRIO_WHO_PROCESS, 0, IOPRIO_IDLE)
        }
    };
    if prioritised != 0 {
        eprintln!(
            "[search] the indexer keeps its disk priority: {}",
            std::io::Error::last_os_error()
        );
    }
}

/// Gives back to the system the memory malloc keeps free (the writer's segments, closed): the
/// service's resident memory while idle is what it holds, not what it once held.
pub fn give_back_memory() {
    #[allow(unsafe_code)]
    // SAFETY: `malloc_trim` takes an integer and only releases free memory of malloc's own.
    unsafe {
        libc::malloc_trim(0);
    }
}

/// The calling thread's scheduling policy and its I/O priority (tests).
#[cfg(test)]
pub(crate) fn current() -> (libc::c_int, libc::c_long) {
    #[allow(unsafe_code)]
    // SAFETY: both calls take integers alone; pid 0 is the calling thread.
    unsafe {
        (
            libc::sched_getscheduler(0),
            libc::syscall(libc::SYS_ioprio_get, IOPRIO_WHO_PROCESS, 0),
        )
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_thread_alone_goes_idle() {
        std::thread::spawn(|| {
            super::idle_thread();
            let (policy, priority) = super::current();
            assert_eq!(policy, libc::SCHED_IDLE);
            assert_eq!(priority >> 13, 3);
            // What it starts inherits it.
            std::thread::spawn(|| assert_eq!(super::current().0, libc::SCHED_IDLE))
                .join()
                .unwrap();
        })
        .join()
        .unwrap();
        assert_ne!(super::current().0, libc::SCHED_IDLE);
    }
}
