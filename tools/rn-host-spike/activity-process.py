#!/usr/bin/env python3
"""A process Activity Monitor's scenarios know (docs/adr/0060-activity-monitor.mdx): named NAME (its
`comm`, what the kernel and Activity Monitor call it), busy for a share BUSY of the time (0: asleep
until a signal comes), and recording each signal it receives, with when it received it on the
monotonic clock the scenarios' steps use, in RECORD:

    activity-process.py NAME BUSY RECORD

SIGTERM is recorded and then ends it, as a program that quits when asked does; SIGKILL cannot be
recorded and is seen by its parent as the way it ended.
"""

import ctypes
import os
import signal
import sys
import time

name, busy, record = sys.argv[1], float(sys.argv[2]), sys.argv[3]
# PR_SET_NAME: the name the kernel gives it (`comm`, at most 15 bytes).
ctypes.CDLL(None, use_errno=True).prctl(15, name.encode()[:15], 0, 0, 0)


def received(number, _frame):
    with open(record, "a") as out:
        out.write(f"{time.monotonic_ns()} {signal.Signals(number).name}\n")
    if number == signal.SIGTERM:
        os._exit(0)


for number in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP, signal.SIGUSR1):
    signal.signal(number, received)
with open(record, "a") as out:
    out.write(f"{time.monotonic_ns()} started {os.getpid()}\n")
if busy <= 0:
    while True:
        signal.pause()
# Busy for BUSY of every tenth of a second, asleep for the rest.
while True:
    until = time.monotonic() + 0.1 * busy
    while time.monotonic() < until:
        pass
    time.sleep(0.1 * (1 - busy))
