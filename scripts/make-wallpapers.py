#!/usr/bin/env python3
"""Writes OxyOS's own wallpapers (docs/adr/0048-wallpapers.mdx): the system catalog the image installs
as /usr/share/oxyos/wallpapers, from nothing but this script.

    python3 scripts/make-wallpapers.py [DESTINATION]

DESTINATION defaults to config/includes.chroot_after_packages/usr/share/oxyos/wallpapers. Every
file is written from this script's numbers alone (no randomness that is not seeded, no time, no
input), so running it again writes the same bytes: the art is reviewed here, as code.

Each wallpaper is SVG, drawn by Skia at the display's size (no raster held larger than the
display): a base linear gradient, soft glows (ellipses filled with radial gradients whose opacity
falls off as a Gaussian, so no edge shows) and translucent ribbons, layered so a wide gradient is
never one flat ramp of a few 8-bit steps. Only rect, ellipse, path, linearGradient and
radialGradient are used, every gradient in the drawing's coordinates (userSpaceOnUse): no filters,
text, external references or embedded rasters.
"""

import colorsys
import json
import math
import pathlib
import random
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
DESTINATION = ROOT / "config/includes.chroot_after_packages/usr/share/oxyos/wallpapers"
WIDTH, HEIGHT = 2560, 1600


# Colours ---------------------------------------------------------------------------------------

def rgb(hex_text):
    hex_text = hex_text.lstrip("#")
    return tuple(int(hex_text[i:i + 2], 16) / 255 for i in (0, 2, 4))


def hexed(colour):
    return "#" + "".join(f"{max(0, min(255, round(c * 255))):02x}" for c in colour)


def hls(h, l, s):
    """A colour from hue (degrees), lightness and saturation (0 to 1)."""
    return hexed(colorsys.hls_to_rgb((h % 360) / 360, max(0, min(1, l)), max(0, min(1, s))))


def hue_of(hex_text):
    h, _, _ = colorsys.rgb_to_hls(*rgb(hex_text))
    return h * 360


def mix(a, b, t):
    ca, cb = rgb(a), rgb(b)
    return hexed(tuple(x + (y - x) * t for x, y in zip(ca, cb)))


# Drawing ---------------------------------------------------------------------------------------

class Drawing:
    def __init__(self):
        self.defs = []
        self.body = []
        self.count = 0

    def _id(self, kind):
        self.count += 1
        return f"{kind}{self.count}"

    def linear(self, stops, x1, y1, x2, y2):
        """A linear gradient in the drawing's coordinates; `stops` are (offset, colour[, opacity])."""
        gid = self._id("l")
        self.defs.append(
            f'<linearGradient id="{gid}" gradientUnits="userSpaceOnUse" x1="{x1:g}" y1="{y1:g}" '
            f'x2="{x2:g}" y2="{y2:g}">{self._stops(stops)}</linearGradient>'
        )
        return gid

    def _stops(self, stops):
        out = []
        for stop in stops:
            offset, colour = stop[0], stop[1]
            opacity = stop[2] if len(stop) > 2 else 1
            extra = f' stop-opacity="{opacity:.3f}"' if opacity < 1 else ""
            out.append(f'<stop offset="{offset:.3f}" stop-color="{colour}"{extra}/>')
        return "".join(out)

    def background(self, stops, angle=90):
        """The whole canvas, a gradient along `angle` degrees (90: top to bottom)."""
        cx, cy = WIDTH / 2, HEIGHT / 2
        a = math.radians(angle)
        reach = abs(math.cos(a)) * WIDTH / 2 + abs(math.sin(a)) * HEIGHT / 2
        gid = self.linear(stops, cx - math.cos(a) * reach, cy - math.sin(a) * reach,
                          cx + math.cos(a) * reach, cy + math.sin(a) * reach)
        self.body.append(f'<rect width="{WIDTH}" height="{HEIGHT}" fill="url(#{gid})"/>')

    def glow(self, x, y, rx, ry, colour, opacity, rotate=0, falloff=3.2):
        """A soft light: an ellipse whose opacity falls off as a Gaussian to nothing at its edge."""
        gid = self._id("g")
        stops = []
        for i in range(9):
            t = i / 8
            a = opacity * (math.exp(-falloff * t * t) - math.exp(-falloff)) / (1 - math.exp(-falloff))
            stops.append((t, colour, a))
        # In the drawing's own coordinates, never the ellipse's bounding box (Skia's SVG module
        # draws a bounding-box gradient on an ellipse as black): a circle of the larger radius at
        # the ellipse's centre, squeezed about that centre to the other radius (the ellipse's
        # rotation applies to its fill too). The matrix stays near the identity: Skia's raster
        # pipeline evaluates a gradient through a large one (a unit circle scaled a thousandfold)
        # at too little precision, in visible steps.
        r = max(rx, ry)
        self.defs.append(
            f'<radialGradient id="{gid}" gradientUnits="userSpaceOnUse" cx="{x:g}" cy="{y:g}" '
            f'r="{r:g}" gradientTransform="translate({x:g} {y:g}) scale({rx / r:.6g} {ry / r:.6g}) '
            f'translate({-x:g} {-y:g})">{self._stops(stops)}</radialGradient>'
        )
        transform = f' transform="rotate({rotate:g} {x:g} {y:g})"' if rotate else ""
        self.body.append(
            f'<ellipse cx="{x:g}" cy="{y:g}" rx="{rx:g}" ry="{ry:g}" fill="url(#{gid})"{transform}/>'
        )

    def ribbon(self, top, bottom, stops, x1, y1, x2, y2):
        """A band between two smooth curves (`top` and `bottom` lists of points, left to right),
        filled with a linear gradient: silk across the canvas."""
        gid = self.linear(stops, x1, y1, x2, y2)
        path = _smooth(top) + " L" + _smooth(list(reversed(bottom)))[1:] + " Z"
        self.body.append(f'<path d="{path}" fill="url(#{gid})"/>')

    def svg(self):
        return (
            f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {WIDTH} {HEIGHT}" '
            f'width="{WIDTH}" height="{HEIGHT}" preserveAspectRatio="xMidYMid slice">'
            f'<defs>{"".join(self.defs)}</defs>{"".join(self.body)}</svg>\n'
        )


def _smooth(points):
    """A path through `points` with Catmull-Rom curves (as cubic Béziers): no corner anywhere."""
    out = [f"M{points[0][0]:.1f} {points[0][1]:.1f}"]
    for i in range(len(points) - 1):
        p0 = points[max(i - 1, 0)]
        p1, p2 = points[i], points[i + 1]
        p3 = points[min(i + 2, len(points) - 1)]
        c1 = (p1[0] + (p2[0] - p0[0]) / 6, p1[1] + (p2[1] - p0[1]) / 6)
        c2 = (p2[0] - (p3[0] - p1[0]) / 6, p2[1] - (p3[1] - p1[1]) / 6)
        out.append(f"C{c1[0]:.1f} {c1[1]:.1f} {c2[0]:.1f} {c2[1]:.1f} {p2[0]:.1f} {p2[1]:.1f}")
    return " ".join(out)


def wave(y, amplitude, phase, length, count=9, x0=-200, x1=WIDTH + 200, tilt=0.0):
    """Points along a gentle wave at height `y` (rising by `tilt` pixels across the canvas)."""
    points = []
    for i in range(count):
        x = x0 + (x1 - x0) * i / (count - 1)
        points.append((x, y - tilt * (x / WIDTH) + amplitude * math.sin(2 * math.pi * x / length + phase)))
    return points


# Silk: the OxyOS wallpaper and the gradients ------------------------------------------------------

def silk(palette, seed):
    """Two silk ribbons crossing a deep field, lit from the upper right. `palette` gives the base's
    top-left and bottom-right colours, the glows' and the ribbons' light and shade."""
    rnd = random.Random(seed)
    d = Drawing()
    d.background([(0, palette["base"][0]), (0.55, palette["base"][1]), (1, palette["base"][2])], angle=32)
    # Broad light from the upper right and depth from the lower left.
    d.glow(WIDTH * 0.86, HEIGHT * 0.08, 1500, 1000, palette["light"], palette["light_opacity"], rotate=-18)
    d.glow(WIDTH * 0.10, HEIGHT * 0.98, 1400, 900, palette["deep"], palette["deep_opacity"], rotate=12)
    d.glow(WIDTH * 0.42, HEIGHT * 0.46, 1300, 700, palette["mid"], palette["mid_opacity"], rotate=-24)
    # The ribbons: wide bands sweeping up to the right, each lit along its length.
    for band, (y, thickness, amplitude, opacity) in enumerate(palette["bands"]):
        phase = rnd.uniform(0, math.tau)
        length = rnd.uniform(2600, 3400)
        top = wave(y, amplitude, phase, length, tilt=520)
        bottom = wave(y + thickness, amplitude * 0.75, phase + 0.35, length * 1.05, tilt=520)
        shade, light = palette["ribbon"][band % len(palette["ribbon"])]
        d.ribbon(top, bottom,
                 [(0, shade, 0), (0.3, shade, opacity * 0.6), (0.68, light, opacity),
                  (1, light, opacity * 0.8)],
                 0, y + thickness, WIDTH, y - 520)
    # A last soft highlight where the ribbons meet the light.
    d.glow(WIDTH * 0.70, HEIGHT * 0.34, 900, 420, palette["sheen"], palette["sheen_opacity"], rotate=-20)
    return d.svg()


def silk_palette(hue, dark, saturation=0.72, neutral=False, lean=28, shade=-18):
    """A silk wallpaper's colours around `hue`: a calm, mid-luminance light variant on which white
    text and the dock's glass read well, and a deep dark one. The light leans `lean` degrees toward
    a neighbouring hue, as light through silk does, and the shade `shade` degrees the other way."""
    s = 0.06 if neutral else saturation
    h2 = hue + (0 if neutral else lean)
    h0 = hue + (0 if neutral else shade)
    if dark:
        return {
            "base": [hls(h0, 0.09, s * 0.9), hls(hue, 0.14, s * 0.85), hls(h2, 0.07, s * 0.8)],
            "light": hls(h2, 0.42, s), "light_opacity": 0.55,
            "deep": hls(h0, 0.03, s), "deep_opacity": 0.7,
            "mid": hls(hue, 0.24, s), "mid_opacity": 0.45,
            "ribbon": [(hls(h0, 0.16, s), hls(h2, 0.52, s * 0.9)), (hls(hue, 0.12, s), hls(hue, 0.42, s))],
            "bands": [(980, 420, 120, 0.42), (1240, 360, 90, 0.30)],
            "sheen": hls(h2, 0.62, s * 0.8), "sheen_opacity": 0.16,
        }
    return {
        "base": [hls(h0, 0.42, s * 0.8), hls(hue, 0.50, s * 0.78), hls(h2, 0.40, s * 0.75)],
        "light": hls(h2, 0.80, s * 0.9), "light_opacity": 0.55,
        "deep": hls(h0, 0.30, s * 0.85), "deep_opacity": 0.55,
        "mid": hls(hue, 0.60, s), "mid_opacity": 0.35,
        "ribbon": [(hls(h0, 0.42, s), hls(h2, 0.84, s * 0.8)), (hls(hue, 0.46, s), hls(hue, 0.76, s * 0.9))],
        "bands": [(980, 420, 120, 0.48), (1240, 360, 90, 0.34)],
        "sheen": hls(h2, 0.92, s * 0.6), "sheen_opacity": 0.22,
    }


# The catalog -------------------------------------------------------------------------------------

# Bloom's accent hues (@oxy.so/bloom's APP_COLOR_PRESETS): each gradient wallpaper is its preset's
# hue, light and dark.
GRADIENTS = [
    ("ocean", {"en": "Ocean", "es": "Océano"}, "#1d9bf0", {}),
    ("iris", {"en": "Iris", "es": "Iris"}, "#5b5bf0", {}),
    ("orchid", {"en": "Orchid", "es": "Orquídea"}, "#c46ede", {}),
    ("coral", {"en": "Coral", "es": "Coral"}, "#f91880", {"lean": 22, "shade": -26}),
    ("ember", {"en": "Ember", "es": "Brasa"}, "#ff7a00", {"saturation": 0.82, "lean": 4, "shade": -24}),
    ("meadow", {"en": "Meadow", "es": "Pradera"}, "#10b981", {"saturation": 0.6}),
    ("lagoon", {"en": "Lagoon", "es": "Laguna"}, "#14b8a6", {"saturation": 0.62}),
    ("graphite", {"en": "Graphite", "es": "Grafito"}, "#607d8b", {"neutral": True}),
]

COLORS = [
    ("#1f5fd6", "Blue", "Azul"),
    ("#3b3fb6", "Indigo", "Índigo"),
    ("#7a3fc2", "Purple", "Morado"),
    ("#c2367a", "Pink", "Rosa"),
    ("#c8333a", "Red", "Rojo"),
    ("#e0702a", "Orange", "Naranja"),
    ("#e8b62e", "Yellow", "Amarillo"),
    ("#2f9a5a", "Green", "Verde"),
    ("#178a8a", "Teal", "Verde azulado"),
    ("#4a4f57", "Graphite", "Grafito"),
    ("#b9bec6", "Silver", "Plata"),
    ("#000000", "Black", "Negro"),
]

LIGHT_DARK = [("light", {"en": "Light", "es": "Claro"}), ("dark", {"en": "Dark", "es": "Oscuro"})]


def write(path, text):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


def manifest(name, category, automatic, preview, variants):
    return json.dumps({"name": name, "category": category, "automatic": automatic,
                       "preview": preview, "variants": variants}, ensure_ascii=False, indent=2) + "\n"


def appearance_wallpaper(root, wallpaper_id, name, category, draw):
    variants = []
    for variant, variant_name in LIGHT_DARK:
        file = f"{variant}.svg"
        write(root / wallpaper_id / file, draw(variant == "dark"))
        variants.append({"id": variant, "name": variant_name, "file": file, "appearance": variant})
    write(root / wallpaper_id / "wallpaper.json", manifest(name, category, "appearance", "light", variants))


def main():
    root = pathlib.Path(sys.argv[1]) if len(sys.argv) > 1 else DESTINATION
    # OxyOS's blue, light and dark (the default is Solar Gradients, examples/make-solar-gradients.rs).
    appearance_wallpaper(root, "oxyos", {"en": "OxyOS", "es": "OxyOS"}, "dynamic",
                         lambda dark: silk(silk_palette(222, dark, saturation=0.74), seed=1))
    for index, (wallpaper_id, name, accent, options) in enumerate(GRADIENTS):
        hue = hue_of(accent)
        appearance_wallpaper(root, wallpaper_id, name, "gradients",
                             lambda dark, hue=hue, options=options, seed=index + 2:
                             silk(silk_palette(hue, dark, **options), seed=seed))
    catalog = {
        "version": 1,
        "default": "solar-gradients",
        "categories": [
            {"id": "dynamic", "name": {"en": "Dynamic Wallpapers", "es": "Fondos dinámicos"}},
            {"id": "gradients", "name": {"en": "Gradients", "es": "Degradados"}},
        ],
        "colors": [{"color": colour, "name": {"en": en, "es": es}} for colour, en, es in COLORS],
    }
    write(root / "catalog.json", json.dumps(catalog, ensure_ascii=False, indent=2) + "\n")


if __name__ == "__main__":
    main()
