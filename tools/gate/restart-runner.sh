#!/bin/sh
# Restarts (or stops) a gate runner only once it is idle (docs/developing.mdx, "The gate's machine").
# A runner stopped under a job leaves that job in progress on GitHub with no one to end it, GitHub
# counting the runner busy and every queued job, the merge queue's among them, waiting behind it.
# Run as root:
#   sh tools/gate/restart-runner.sh N [restart|stop]
# It waits until runner N has no job (no Runner.Worker in its service) twice in a row, a few seconds
# apart, then restarts or stops it at once, waiting at most WAIT_LIMIT seconds (3 hours by default).
# A job handed to the runner in that moment is cancelled by the runner itself and said so, never left
# behind; the gate's watchdog (tools/gate/watchdog.py) ends one that is anyway. Never move a runner's
# directory or cache, or register it again, but through this: stopped first.
set -eu
[ "$(id -u)" = 0 ] || { echo "restart-runner.sh: run as root" >&2; exit 1; }
n=${1:?runner N}
action=${2:-restart}
case $n in *[!0-9]* | 0*) echo "restart-runner.sh: runner N, a number" >&2; exit 2 ;; esac
case $action in restart | stop) ;; *) echo "restart-runner.sh: restart or stop" >&2; exit 2 ;; esac
unit=oxyos-gate-runner@$n.service
limit=${WAIT_LIMIT:-10800}
busy() {
  for pid in $(cat "/sys/fs/cgroup/oxyosgate.slice/oxyosgate-$n.slice/$unit/cgroup.procs" 2>/dev/null); do
    case $(cat "/proc/$pid/comm" 2>/dev/null) in Runner.Worker) return 0 ;; esac
  done
  return 1
}
waited=0
while busy || { sleep 5; busy; }; do
  [ "$waited" -lt "$limit" ] || { echo "restart-runner.sh: runner $n still busy after ${limit}s; nothing done" >&2; exit 1; }
  [ "$waited" -gt 0 ] || echo "restart-runner.sh: runner $n is running a job; waiting for it to end"
  sleep 20; waited=$((waited + 25))
done
systemctl "$action" "$unit"
echo "restart-runner.sh: runner $n idle, ${action}ed"
