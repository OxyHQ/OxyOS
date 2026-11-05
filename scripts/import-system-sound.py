#!/usr/bin/env python3
"""Imports a sound the owner supplied as one of the system's sounds (docs/adr/0044-system-sounds.mdx).

The source is decoded to 16-bit PCM at its own rate and channels (ffmpeg), the silence before and
after the sound is cut away sample-exactly (nothing between the first and the last sample louder
than THRESHOLD dBFS is changed: no fade, no gain, no resampling), and the rest is encoded as Ogg
Vorbis at libvorbis' highest quality (-q 10), the freedesktop.org sound theme's format, which the
session decodes (crates/oxyos-sound). The cut falls on the nearest sample at most one step from
zero. `--before SECONDS` ignores what comes after that point (a click a source ends with).

Usage: import-system-sound.py SOURCE OUTPUT.oga [--threshold DB] [--before SECONDS]

A developer tool: ffmpeg is not part of the image.
"""

import argparse
import struct
import subprocess
import sys
import tempfile
import wave
from pathlib import Path


def cut(source: Path, kept: Path, threshold_db: float, before: float | None) -> str:
    with wave.open(str(source)) as w:
        frames, channels, rate = w.getnframes(), w.getnchannels(), w.getframerate()
        if w.getsampwidth() != 2:
            raise SystemExit(f"{source}: not 16-bit PCM")
        raw = w.readframes(frames)
    samples = struct.unpack(f"<{frames * channels}h", raw)
    loudest = 32768 * 10 ** (threshold_db / 20)
    end = min(frames, int(before * rate)) if before else frames

    def loud(i):
        return any(abs(samples[i * channels + c]) > loudest for c in range(channels))

    def still(i):
        return all(abs(samples[i * channels + c]) <= 1 for c in range(channels))

    first = next(i for i in range(end) if loud(i))
    while first > 0 and not still(first):
        first -= 1
    last = max(i for i in range(end) if loud(i))
    while last < end - 1 and not still(last):
        last += 1
    with wave.open(str(kept), "wb") as out:
        out.setnchannels(channels)
        out.setsampwidth(2)
        out.setframerate(rate)
        out.writeframes(raw[first * channels * 2:(last + 1) * channels * 2])
    return (f"frames {first} to {last} of {frames} kept ({rate} Hz, {channels} channels): "
            f"{first / rate * 1000:.1f} ms cut before, {(frames - 1 - last) / rate * 1000:.1f} ms after")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("source", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--threshold", type=float, default=-80.0)
    parser.add_argument("--before", type=float)
    args = parser.parse_args()
    with tempfile.TemporaryDirectory() as scratch:
        decoded, kept = Path(scratch) / "decoded.wav", Path(scratch) / "kept.wav"
        subprocess.run(["ffmpeg", "-hide_banner", "-loglevel", "error", "-y", "-i", str(args.source),
                        "-c:a", "pcm_s16le", str(decoded)], check=True)
        said = cut(decoded, kept, args.threshold, args.before)
        subprocess.run(["ffmpeg", "-hide_banner", "-loglevel", "error", "-y", "-i", str(kept),
                        "-map_metadata", "-1", "-fflags", "+bitexact", "-flags:a", "+bitexact", "-c:a", "libvorbis", "-q:a", "10",
                        str(args.output)], check=True)
    print(f"{args.source} -> {args.output}: {said}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
