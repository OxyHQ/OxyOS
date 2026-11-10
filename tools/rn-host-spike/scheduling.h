// The host's input path ahead of batch work (docs/adr/0066-quality-of-service.mdx, decision 2): the
// main thread (the UI thread: input, mounting, drawing, presenting) and the JavaScript thread
// (`oxyos-js`, js-thread.h: React, Fabric's commits) each ask the scheduler for a 1 ms slice (EEVDF, the fair scheduler since Linux 6.6, takes `sched_setattr`'s
// `sched_runtime` on SCHED_OTHER as the thread's slice since 6.12): a shorter request is an earlier
// virtual deadline, so a thread woken by input preempts batch work at its default slice instead of
// waiting for it to run out. No privilege is needed, and its policy and nice are kept. It sets
// SCHED_RESET_ON_FORK, so the threads the host starts afterwards (Hermes' collector, Skia's, the
// workers) get the default slice: only the threads input waits on ask. A later change of either
// thread's policy must keep that flag (an unprivileged thread cannot clear it), as oxyos-priority's
// request_slice keeps it.
#pragma once

namespace oxyos {

// The calling thread's slice: first thing in main (the UI thread), and as the JavaScript thread
// starts. A refusal (a policy without a slice; an older kernel ignores the request) is said and
// changes nothing else.
void requestInteractiveSlice();

}  // namespace oxyos
