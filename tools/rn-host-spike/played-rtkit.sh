#!/bin/sh
# rtkit played for a run of the development compositor (docs/adr/0066-quality-of-service.mdx,
# decision 2; docs/performance.mdx): the high priority the system's rtkit gives a session on a seat
# (nice -15 on its turn thread), which polkit refuses a process in no seat session, given to the
# run's compositor so the run is measured as a session runs. The compositor asks it as a session
# asks the system's (`OXYOS_RTKIT_BUS` names its bus, never the system's).
#
# The played service (`oxyos-offscreen --play-rtkit`) alone holds CAP_SYS_NICE, in a transient
# system unit of its own (ambient, its bounding set nothing else, no new privileges, no devices,
# the system read-only), as the caller's user: never the compositor, a host or an application. It
# answers as rtkit 0.13 does (the caller's own thread, nice no lower than -15, SCHED_RESET_ON_FORK)
# and changes only threads of the run's control group, so it can raise nothing of the machine's
# own. Every answer goes to DIR/played-rtkit.log.
#
# Usage: played-rtkit.sh start OFFSCREEN BUS CGROUP DIR   prints the unit's name once it serves
#          BUS     the run's bus (a dbus-daemon of the run's own: unix:path=...)
#          CGROUP  the run's control group (/sys/fs/cgroup/..., or as /proc/self/cgroup says it)
#          DIR     a directory of the run's own: the log, and the file saying it serves
#        played-rtkit.sh stop UNIT
set -eu
fail() { echo "played-rtkit: $*" >&2; exit 1; }
case ${1:-} in
  start)
    [ $# = 5 ] || fail "usage: played-rtkit.sh start OFFSCREEN BUS CGROUP DIR"
    offscreen=$(cd "$(dirname "$2")" && pwd)/$(basename "$2") bus=$3 cgroup=$4 dir=$(cd "$5" && pwd)
    [ -x "$offscreen" ] || fail "no $offscreen"
    sudo -n true 2>/dev/null || fail "needs sudo without a password (the service's unit is a system one)"
    unit=oxyos-played-rtkit-$$
    ready=$dir/played-rtkit.ready
    rm -f "$ready"
    sudo -n systemd-run --quiet --collect --unit="$unit" --uid="$(id -u)" --gid="$(id -g)" \
      -p AmbientCapabilities=CAP_SYS_NICE -p CapabilityBoundingSet=CAP_SYS_NICE -p NoNewPrivileges=yes \
      -p PrivateDevices=yes -p ProtectSystem=strict -p ProtectHome=read-only -p ReadWritePaths="$dir" \
      -p StandardOutput=append:"$dir/played-rtkit.log" -p StandardError=append:"$dir/played-rtkit.log" \
      -- "$offscreen" --play-rtkit "$bus" "$cgroup" "$ready"
    i=0
    until [ -e "$ready" ]; do
      systemctl is-active -q "$unit" || fail "it did not start: $(tail -n 5 "$dir/played-rtkit.log" 2>/dev/null)"
      i=$((i + 1)); [ $i -lt 100 ] || { sudo -n systemctl stop "$unit"; fail "it did not serve within 10 s"; }
      sleep 0.1
    done
    echo "$unit"
    ;;
  stop)
    [ $# = 2 ] || fail "usage: played-rtkit.sh stop UNIT"
    case $2 in oxyos-played-rtkit-*) ;; *) fail "not a played rtkit's unit: $2" ;; esac
    sudo -n systemctl stop "$2" 2>/dev/null || true
    ;;
  *) fail "usage: played-rtkit.sh start OFFSCREEN BUS CGROUP DIR | stop UNIT" ;;
esac
