#!/usr/bin/env python3
"""Makes the repository's test JPEG, assets/test-images/p3-5120x2880.jpg: a 5120 x 2880 picture in
saturated colours (FFmpeg's gradients source with a little seeded grain, so it is large enough to
be cut short and detailed enough to reduce) carrying a Display P3 ICC profile (moxcms's, written
by `cargo run -p oxyos-image --example display-p3-profile`), so a decoder that ignores the profile
draws other colours. It is made from nothing but the repository, its dependencies and FFmpeg, and
replaces the wallpaper frame the tests used to read (docs/adr/0048-wallpapers.mdx, decision 8).

    python3 scripts/make-test-images.py
"""

import pathlib
import struct
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parents[1]
OUT = ROOT / "assets" / "test-images" / "p3-5120x2880.jpg"


def with_profile(jpeg: bytes, profile: bytes) -> bytes:
    """The JPEG with `profile` in an APP2 ICC_PROFILE segment right after its SOI (one chunk)."""
    assert jpeg[:2] == b"\xff\xd8", "not a JPEG"
    payload = b"ICC_PROFILE\0" + bytes([1, 1]) + profile
    assert len(payload) + 2 <= 0xFFFF, "the profile fits one segment"
    return jpeg[:2] + b"\xff\xe2" + struct.pack(">H", len(payload) + 2) + payload + jpeg[2:]


def main() -> int:
    profile = subprocess.run(
        ["cargo", "run", "-q", "--locked", "-p", "oxyos-image", "--example", "display-p3-profile"],
        cwd=ROOT, check=True, capture_output=True,
    ).stdout
    with tempfile.TemporaryDirectory() as scratch:
        plain = pathlib.Path(scratch) / "plain.jpg"
        subprocess.run(
            ["ffmpeg", "-v", "error", "-y", "-f", "lavfi", "-i",
             "gradients=s=5120x2880:n=4:c0=0x1f4fd8:c1=0xff2d6f:c2=0x00c08b:c3=0xffb000"
             ":x0=0:y0=0:x1=5120:y1=2880:type=radial:speed=0:seed=48",
             "-vf", "noise=alls=10:allf=u:all_seed=48,format=yuvj420p",
             "-frames:v", "1", "-q:v", "2", str(plain)],
            check=True,
        )
        OUT.parent.mkdir(parents=True, exist_ok=True)
        OUT.write_bytes(with_profile(plain.read_bytes(), profile))
    print(f"{OUT.relative_to(ROOT)}: {OUT.stat().st_size} bytes")
    return 0


if __name__ == "__main__":
    sys.exit(main())
