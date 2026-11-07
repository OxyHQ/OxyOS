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
never one flat ramp of a few 8-bit steps. Only rect, ellipse, circle, path, linearGradient and
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

    def land(self, points, stops, x1, y1, x2, y2):
        """A smooth ridge from the left edge to the right, filled down to the bottom."""
        gid = self.linear(stops, x1, y1, x2, y2)
        path = _smooth(points) + f" L{WIDTH + 40} {HEIGHT + 40} L-40 {HEIGHT + 40} Z"
        self.body.append(f'<path d="{path}" fill="url(#{gid})"/>')

    def dot(self, x, y, r, colour, opacity):
        self.body.append(
            f'<circle cx="{x:.1f}" cy="{y:.1f}" r="{r:.2f}" fill="{colour}" fill-opacity="{opacity:.2f}"/>'
        )

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


# Horizon: a sky through the day ------------------------------------------------------------------

# Each moment: the sky from top to horizon, where the sun's light is (x, y as a share of the
# canvas, its colour and strength), and the three ridges from far to near, each top and foot.
HORIZON = [
    ("night", {"en": "Night", "es": "Noche"}, (-30.0, 0.0), {
        "sky": ["#05081a", "#0b1434", "#1a2552"], "sun": (0.70, 0.95, "#3a4f9a", 0.35),
        "ridges": [("#151d3f", "#0e1530"), ("#0d1430", "#090e22"), ("#070b1c", "#04060f")],
        "stars": 0.85, "haze": ("#2c3b78", 0.25)}),
    ("dawn", {"en": "Dawn", "es": "Alba"}, (-12.0, 90.0), {
        "sky": ["#121a44", "#3b3d7a", "#b06b8f"], "sun": (0.22, 1.02, "#f0a27a", 0.55),
        "ridges": [("#45406f", "#2f2c56"), ("#2b2850", "#1d1b3c"), ("#17152d", "#0e0d1f")],
        "stars": 0.35, "haze": ("#d7899c", 0.30)}),
    ("sunrise", {"en": "Sunrise", "es": "Amanecer"}, (0.0, 90.0), {
        "sky": ["#3c5aa6", "#d98a8a", "#ffc58a"], "sun": (0.22, 0.86, "#ffe2a8", 0.85),
        "ridges": [("#8a6f9a", "#6b5683"), ("#5a4677", "#41325c"), ("#302543", "#1f182e")],
        "stars": 0.0, "haze": ("#ffd2a0", 0.40)}),
    ("morning", {"en": "Morning", "es": "Mañana"}, (15.0, 90.0), {
        "sky": ["#4f8be0", "#8cb8ef", "#dcebf7"], "sun": (0.26, 0.50, "#fff6dc", 0.75),
        "ridges": [("#8fa9d4", "#7894c4"), ("#5f7fb4", "#4a6aa0"), ("#3a5488", "#2a406c")],
        "stars": 0.0, "haze": ("#eef5ff", 0.35)}),
    ("day", {"en": "Day", "es": "Día"}, (60.0, 180.0), {
        "sky": ["#2f73dc", "#5b9cf0", "#a9d1fb"], "sun": (0.55, 0.06, "#ffffff", 0.70),
        "ridges": [("#7aa4e0", "#6290d4"), ("#4b78c4", "#3a63ad"), ("#2d4f93", "#213c75")],
        "stars": 0.0, "haze": ("#d6ebff", 0.30)}),
    ("afternoon", {"en": "Afternoon", "es": "Tarde"}, (15.0, 270.0), {
        "sky": ["#3f74c9", "#9bb7e3", "#f3dcc0"], "sun": (0.76, 0.50, "#fff0d0", 0.75),
        "ridges": [("#9aa6cf", "#8290bf"), ("#6a74a8", "#535d91"), ("#3f4675", "#2e335b")],
        "stars": 0.0, "haze": ("#ffe8cf", 0.35)}),
    ("sunset", {"en": "Sunset", "es": "Puesta de sol"}, (0.0, 270.0), {
        "sky": ["#2d3a86", "#c25b7a", "#ff9f5a"], "sun": (0.78, 0.86, "#ffc77a", 0.90),
        "ridges": [("#8b4f78", "#733f66"), ("#55325f", "#41264b"), ("#2a1a34", "#1b1124")],
        "stars": 0.0, "haze": ("#ff9e72", 0.40)}),
    ("dusk", {"en": "Dusk", "es": "Anochecer"}, (-12.0, 270.0), {
        "sky": ["#0c1236", "#2c2a68", "#7a4f8a"], "sun": (0.80, 1.02, "#c77a9a", 0.50),
        "ridges": [("#382f63", "#28224d"), ("#211c43", "#171433"), ("#110e24", "#0a0816")],
        "stars": 0.45, "haze": ("#8f5c9c", 0.28)}),
]


def horizon(moment):
    sky, sun, ridges = moment["sky"], moment["sun"], moment["ridges"]
    d = Drawing()
    d.background([(0, sky[0]), (0.55, sky[1]), (0.82, sky[2]), (1, sky[2])], angle=90)
    if moment["stars"]:
        rnd = random.Random(7)
        for _ in range(140):
            x, y = rnd.uniform(0, WIDTH), rnd.uniform(0, HEIGHT * 0.62) ** 1.0
            fade = (1 - y / (HEIGHT * 0.62)) ** 1.5
            d.dot(x, y, rnd.uniform(0.9, 2.4), "#ffffff", moment["stars"] * fade * rnd.uniform(0.25, 0.9))
    sx, sy, colour, strength = sun
    d.glow(WIDTH * sx, HEIGHT * sy, 1700, 1000, colour, strength * 0.65, falloff=2.6)
    d.glow(WIDTH * sx, HEIGHT * sy, 520, 420, colour, strength, falloff=3.6)
    haze, haze_opacity = moment["haze"]
    d.glow(WIDTH * 0.5, HEIGHT * 0.80, 2200, 380, haze, haze_opacity, falloff=2.4)
    # Three ridges from far to near, each a little darker toward its foot.
    shapes = [(1060, 70, 1.1, 2400, 120), (1180, 95, 2.6, 2900, -80), (1330, 80, 4.2, 3300, 60)]
    for (top, foot), (y, amplitude, phase, length, tilt) in zip(ridges, shapes):
        d.land(wave(y, amplitude, phase, length, count=11, tilt=tilt), [(0, top), (1, foot)],
               0, y - amplitude, 0, HEIGHT)
    return d.svg()


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
    # The default: OxyOS's blue, light and dark.
    appearance_wallpaper(root, "oxyos", {"en": "OxyOS", "es": "OxyOS"}, "dynamic",
                         lambda dark: silk(silk_palette(222, dark, saturation=0.74), seed=1))
    # Horizon follows the sun (ADR 0012's rule over these anchors).
    variants = []
    for variant, variant_name, (elevation, azimuth), moment in HORIZON:
        file = f"{variant}.svg"
        write(root / "horizon" / file, horizon(moment))
        variants.append({"id": variant, "name": variant_name, "file": file,
                         "sun": {"elevation": elevation, "azimuth": azimuth}})
    write(root / "horizon" / "wallpaper.json",
          manifest({"en": "Horizon", "es": "Horizonte"}, "dynamic", "sun", "day", variants))
    for index, (wallpaper_id, name, accent, options) in enumerate(GRADIENTS):
        hue = hue_of(accent)
        appearance_wallpaper(root, wallpaper_id, name, "gradients",
                             lambda dark, hue=hue, options=options, seed=index + 2:
                             silk(silk_palette(hue, dark, **options), seed=seed))
    catalog = {
        "version": 1,
        "default": "oxyos",
        "categories": [
            {"id": "dynamic", "name": {"en": "Dynamic Wallpapers", "es": "Fondos dinámicos"}},
            {"id": "gradients", "name": {"en": "Gradients", "es": "Degradados"}},
        ],
        "colors": [{"color": colour, "name": {"en": en, "es": es}} for colour, en, es in COLORS],
    }
    write(root / "catalog.json", json.dumps(catalog, ensure_ascii=False, indent=2) + "\n")


if __name__ == "__main__":
    main()
