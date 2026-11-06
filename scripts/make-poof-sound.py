#!/usr/bin/env python3
"""Synthesizes the poof's sound (docs/adr/0044-system-sounds.mdx, docs/adr/0035-drag-and-drop.mdx):
the soft puff of air an item dragged out of the dock vanishes with. No sound was supplied for it, so
it is made here, from nothing but this script, the same bytes every run: white noise from a fixed
seed, through a low-pass filter whose cutoff falls from 3 kHz to 300 Hz (the puff's breath closing),
shaped by an 8 ms rise and an exponential fall over a quarter second (its last 20 ms closing to
silence), peaking at -14 dBFS, at 48 kHz mono, 16-bit. The WAV it writes is then imported as every system sound is:

    scripts/make-poof-sound.py poof.wav
    scripts/import-system-sound.py poof.wav config/includes.chroot_after_packages/usr/share/sounds/oxyos/stereo/poof.oga

Usage: make-poof-sound.py OUTPUT.wav
"""

import math
import random
import struct
import sys
import wave

RATE = 48_000
SECONDS = 0.25
ATTACK = 0.008
PEAK_DB = -14.0
SEED = 0x0F0F


def main() -> int:
    if len(sys.argv) != 2:
        print(__doc__, file=sys.stderr)
        return 2
    noise = random.Random(SEED)
    frames = int(RATE * SECONDS)
    filtered = []
    state = 0.0
    for i in range(frames):
        t = i / RATE
        share = t / SECONDS
        # The cutoff falls exponentially from 3 kHz to 300 Hz: bright as it bursts, then dull.
        cutoff = 3000.0 * (300.0 / 3000.0) ** share
        alpha = 1.0 - math.exp(-2.0 * math.pi * cutoff / RATE)
        state += alpha * (noise.uniform(-1.0, 1.0) - state)
        envelope = min(1.0, t / ATTACK) * math.exp(-(max(0.0, t - ATTACK)) / 0.055)
        # Its last 20 ms close to silence on a half cosine, so it ends without a click.
        left = SECONDS - t
        if left < 0.02:
            envelope *= 0.5 - 0.5 * math.cos(math.pi * left / 0.02)
        filtered.append(state * envelope)
    # Normalized to its peak, then to -14 dBFS: a soft sound beside the Trash's.
    loudest = max(abs(sample) for sample in filtered) or 1.0
    gain = 10 ** (PEAK_DB / 20) * 32767 / loudest
    samples = [max(-32768, min(32767, round(sample * gain))) for sample in filtered]
    with wave.open(sys.argv[1], "wb") as out:
        out.setnchannels(1)
        out.setsampwidth(2)
        out.setframerate(RATE)
        out.writeframes(struct.pack(f"<{len(samples)}h", *samples))
    print(f"{sys.argv[1]}: {frames} frames, {SECONDS} s at {RATE} Hz mono")
    return 0


if __name__ == "__main__":
    sys.exit(main())
