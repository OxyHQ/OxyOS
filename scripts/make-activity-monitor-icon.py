#!/usr/bin/env python3
"""Draws Activity Monitor's icon (tools/shell-preview/assets/apps/activity-monitor.png).

Art as code, so it is reviewed here and drawn the same on every machine: a 512 x 512 macOS-style
application icon. A graphite tile in Apple's continuous-corner shape (a superellipse on the macOS
icon grid, 412 px with a soft shadow under it) holds a dark screen with a faint grid, across which a
bright green CPU history runs as a monitor's trace: its line, its glow and the area under it.

Pure Python (no PIL here): every shape is a signed distance, antialiased by its distance to the
edge over one pixel, composited in premultiplied light and written as an 8-bit RGBA PNG with zlib.

    python3 scripts/make-activity-monitor-icon.py [OUT]
"""

import itertools
import math
import struct
import sys
import zlib
from pathlib import Path

SIZE = 512
REPOSITORY = Path(__file__).resolve().parent.parent
DEFAULT_OUT = REPOSITORY / "tools/shell-preview/assets/apps/activity-monitor.png"

# The macOS icon grid at 512: an 824/1024 tile, centred, its corners a superellipse.
TILE_HALF = 206.0
TILE_CENTRE = (256.0, 256.0)
TILE_EXPONENT = 5.0

# The screen inside the tile, and its corner radius.
SCREEN = (100.0, 116.0, 412.0, 396.0)
SCREEN_RADIUS = 30.0

# The trace: a CPU history over the screen, left to right, as shares of its height.
TRACE = [0.22, 0.30, 0.25, 0.38, 0.33, 0.52, 0.36, 0.44, 0.80, 0.48, 0.41, 0.60, 0.52, 0.68, 0.57, 0.64]
TRACE_INSET = 26.0
LINE_WIDTH = 7.0
GREEN = (0.20, 0.92, 0.42)


def clamp(value, low=0.0, high=1.0):
    return low if value < low else high if value > high else value


def coverage(distance):
    """How much of a pixel a shape covers, from its signed distance (negative inside)."""
    return clamp(0.5 - distance)


def smoothstep(edge0, edge1, x):
    t = clamp((x - edge0) / (edge1 - edge0))
    return t * t * (3.0 - 2.0 * t)


def superellipse_distance(x, y, centre, half, exponent):
    """The signed distance to a superellipse, its implicit function over its gradient's length."""
    px = abs(x - centre[0]) / half
    py = abs(y - centre[1]) / half
    if px == 0.0 and py == 0.0:
        return -half
    s = px**exponent + py**exponent
    g = s ** (1.0 / exponent) - 1.0
    common = s ** (1.0 / exponent - 1.0) / half
    gx = common * px ** (exponent - 1.0)
    gy = common * py ** (exponent - 1.0)
    return g / max(1e-9, math.hypot(gx, gy))


def rounded_rect_distance(x, y, rect, radius):
    left, top, right, bottom = rect
    cx, cy = (left + right) / 2.0, (top + bottom) / 2.0
    hx, hy = (right - left) / 2.0 - radius, (bottom - top) / 2.0 - radius
    qx, qy = abs(x - cx) - hx, abs(y - cy) - hy
    outside = math.hypot(max(qx, 0.0), max(qy, 0.0))
    return outside + min(max(qx, qy), 0.0) - radius


def trace_points():
    left, top, right, bottom = SCREEN
    # Edge to edge, as a monitor's trace runs on: the screen's corners clip it.
    x0, x1 = left - LINE_WIDTH, right + LINE_WIDTH
    y0, y1 = top + TRACE_INSET + 30.0, bottom - TRACE_INSET
    step = (x1 - x0) / (len(TRACE) - 1)
    return [(x0 + i * step, y1 - share * (y1 - y0)) for i, share in enumerate(TRACE)]


POINTS = trace_points()


def trace_y(x):
    """The trace's height at x, or None outside it."""
    if x < POINTS[0][0] or x > POINTS[-1][0]:
        return None
    for (ax, ay), (bx, by) in itertools.pairwise(POINTS):
        if ax <= x <= bx:
            return ay + (by - ay) * (x - ax) / (bx - ax)
    return POINTS[-1][1]


def segment_distance(x, y, a, b):
    ax, ay = a
    bx, by = b
    dx, dy = bx - ax, by - ay
    t = clamp(((x - ax) * dx + (y - ay) * dy) / (dx * dx + dy * dy))
    return math.hypot(x - (ax + t * dx), y - (ay + t * dy))


def trace_distance(x, y):
    """The distance to the trace's polyline, from the segments near x alone."""
    best = 1e9
    for a, b in itertools.pairwise(POINTS):
        if a[0] - 40.0 <= x <= b[0] + 40.0:
            best = min(best, segment_distance(x, y, a, b))
    return best


def over(pixel, colour, alpha):
    """Composites `colour` at `alpha` over a premultiplied pixel."""
    if alpha <= 0.0:
        return pixel
    r, g, b, a = pixel
    keep = 1.0 - alpha
    return (colour[0] * alpha + r * keep, colour[1] * alpha + g * keep, colour[2] * alpha + b * keep, alpha + a * keep)


def mix(a, b, t):
    return tuple(a[i] + (b[i] - a[i]) * t for i in range(3))


def shade(x, y):
    px, py = x + 0.5, y + 0.5
    pixel = (0.0, 0.0, 0.0, 0.0)

    # The shadow under the tile: soft, a little below it, as a Mac's icons sit on the Dock.
    shadow = superellipse_distance(px, py - 6.0, TILE_CENTRE, TILE_HALF, TILE_EXPONENT)
    pixel = over(pixel, (0.0, 0.0, 0.0), 0.32 * (1.0 - smoothstep(-10.0, 14.0, shadow)))

    tile = superellipse_distance(px, py, TILE_CENTRE, TILE_HALF, TILE_EXPONENT)
    inside_tile = coverage(tile)
    if inside_tile <= 0.0:
        return pixel

    # The tile: graphite at the top to near black at the bottom.
    top, bottom = TILE_CENTRE[1] - TILE_HALF, TILE_CENTRE[1] + TILE_HALF
    t = clamp((py - top) / (bottom - top))
    tile_colour = mix((0.29, 0.30, 0.33), (0.06, 0.06, 0.07), t)
    pixel = over(pixel, tile_colour, inside_tile)

    # The screen, set into the tile, with a hairline of light along its lower lip.
    screen = rounded_rect_distance(px, py, SCREEN, SCREEN_RADIUS)
    inside_screen = coverage(screen)
    lip = rounded_rect_distance(px, py - 1.5, SCREEN, SCREEN_RADIUS)
    pixel = over(pixel, (1.0, 1.0, 1.0), 0.10 * clamp(1.0 - abs(lip) / 1.2) * inside_tile * (1.0 - inside_screen))
    if inside_screen > 0.0:
        st = clamp((py - SCREEN[1]) / (SCREEN[3] - SCREEN[1]))
        pixel = over(pixel, mix((0.035, 0.06, 0.05), (0.01, 0.02, 0.015), st), inside_screen)

        # The grid: faint green lines every 39 px, as an oscilloscope's graticule.
        gx = (px - SCREEN[0]) % 39.0
        gy = (py - SCREEN[1]) % 35.0
        line = max(clamp(1.0 - abs(gx - 19.5) / 0.7), clamp(1.0 - abs(gy - 17.5) / 0.7))
        pixel = over(pixel, GREEN, 0.10 * line * inside_screen)

        # The area under the trace, fading toward the screen's bottom.
        ty = trace_y(px)
        if ty is not None and py > ty:
            fade = 1.0 - clamp((py - ty) / (SCREEN[3] - TRACE_INSET - ty + 1.0))
            edge = clamp(py - ty)
            pixel = over(pixel, GREEN, (0.10 + 0.26 * fade) * edge * inside_screen)

        # The trace: its glow, then its line.
        distance = trace_distance(px, py)
        if distance < 40.0:
            glow = math.exp(-(distance * distance) / (2.0 * 9.0 * 9.0))
            pixel = over(pixel, GREEN, 0.45 * glow * inside_screen)
            line_cover = coverage(distance - LINE_WIDTH / 2.0)
            pixel = over(pixel, mix(GREEN, (0.85, 1.0, 0.88), 0.35), line_cover * inside_screen)

        # The screen's glass: a sheen over its upper part.
        sheen = clamp(1.0 - st * 1.8) * 0.06
        pixel = over(pixel, (1.0, 1.0, 1.0), sheen * inside_screen)

    # The tile's soft inner border: light along the top, fading down its sides.
    rim = clamp(1.0 - abs(tile + 1.25) / 1.25)
    pixel = over(pixel, (1.0, 1.0, 1.0), rim * (0.22 * (1.0 - t) + 0.05) * inside_tile)
    return pixel


def png(width, height, rows):
    def chunk(kind, data):
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body) & 0xFFFFFFFF)

    raw = b"".join(b"\x00" + row for row in rows)
    header = struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b"")


def main():
    out = Path(sys.argv[1]) if len(sys.argv) > 1 else DEFAULT_OUT
    rows = []
    for y in range(SIZE):
        row = bytearray()
        for x in range(SIZE):
            r, g, b, a = shade(x, y)
            if a > 0.0:
                # Premultiplied to straight alpha, as PNG stores it.
                r, g, b = r / a, g / a, b / a
            row += bytes(round(clamp(v) * 255.0) for v in (r, g, b, a))
        rows.append(bytes(row))
    out.write_bytes(png(SIZE, SIZE, rows))
    print(out)


if __name__ == "__main__":
    main()
