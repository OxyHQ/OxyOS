#!/usr/bin/env python3
"""Visual fidelity of the OxyOS Bloom host against Bloom in a browser, component by component.

The same gallery of Bloom's components (sample/FidelityGallery.tsx, its pages in
sample/fidelity-pages.json) is drawn twice: by Chromium, built for the web as an Oxy application ships
Bloom to a browser (react-native-web, fidelity/web.mjs), and by the native Bloom host (React
Native's Android implementation, tools/rn-host-spike) on the offscreen compositor (regress.py,
scenarios/fidelity*.json). Each component sits in a cell of its own at a fixed place (a multiple of
4 logical pixels, whole at 1.25), and both sides report where every cell and its content were laid
out (the gallery's `onLayout`: `window.__fidelity` in the browser, a `[fidelity]` line in the
host's log). Each cell is cropped from both frames at the same rectangle and compared:

- mae: the mean absolute difference of the RGB channels (0-255);
- over: the share of pixels with a channel more than `tolerance` levels apart (percent);
- ssim: the structural similarity of their luminance (Rec. 601), the mean over 8x8 windows a
  stride of 4 apart (one window for a smaller crop), C1 = (0.01*255)^2, C2 = (0.03*255)^2;
  the same numbers with numpy or without it (plain Python: a whole report in about a minute).

A cell's content laid out at another size on the host than in the browser is a measurement
difference (text measured otherwise, a component sized otherwise), reported apart from painting.

Both sides draw with the same font files: the web build's @font-face rules name the `.ttf` files
the host registers (Bloom's fonts), so differences are drawing, never font choice. Each side runs
light and dark (the browser's prefers-color-scheme; the session's `appearance`), at scale 1 and
1.25 (deviceScaleFactor; the output's scale), motion reduced and animations settled.

Usage:

  fidelity.py run --offscreen BIN --runtimes DIR --packages DIR --out DIR [--bloom DIR]
                  [--pages 1,2] [--configs light@1,dark@1.25] [--jobs N] [--check]
      builds the web export, shoots it in Chromium, runs the host's scenarios and writes the report:
      OUT/report.html (worst first: web | host | difference) and OUT/scores.json. --packages is
      what sample/packages.mjs made (its probes/ hold the gallery), --runtimes the runtime it was
      built for (`cmake --install`), --bloom the Bloom checkout the probes were built from (BLOOM_DIR
      by default; the web build uses the same). With --check, exits 1 when a cell passes its
      threshold (below).
  fidelity.py web --out DIR [--bloom DIR] [--pages ..] [--configs ..]       the browser's half
  fidelity.py host --offscreen BIN --runtimes DIR --packages DIR --out DIR [--pages ..] [--configs ..]
  fidelity.py report --out DIR [--check]           compares what OUT holds and writes the report
  fidelity.py baseline --out DIR                   records OUT's scores in the thresholds file as
                                                   each cell's floor (status "baseline")

Thresholds are scenarios/fidelity-thresholds.json: `default` for every cell, `cells` by id (the
same for every scheme and scale), each with `ssim_min`, `over_max` (percent) and `mae_max`, and
`expected` cells (a paragraph whose lines break otherwise: Android's high-quality line breaking
against the browser's greedy one) reported but never failed. A cell missing on either side fails.
`target` is what a faithful renderer reaches; `baseline` records the numbers of the host the floors
were taken from.

The web half needs `npm ci` in sample/ (Tailwind) and in fidelity/ (playwright-core, which drives
the Chromium Playwright installed in ~/.cache/ms-playwright), and Bloom's node_modules (Expo,
react-native-web). The host half runs regress.py, so it needs what regress.py needs (the offscreen
compositor's libraries on the library path). Pages 7 and 8 (overlays) and 9 (the app shell in a
whole window) are each one cell over the window; Bloom draws their overlays otherwise on native by
design, so they are `expected` and read for their parts.

Example (a host built and packaged as tools/rn-host-spike/build.sh and sample/packages.mjs do):

  python3 tools/rn-host-spike/fidelity.py run --offscreen target/release/oxyos-offscreen \\
      --runtimes MACHINE/runtimes --packages MACHINE/packages --bloom ~/Oxy/Bloom --out OUT --check
"""

import argparse
import concurrent.futures
import datetime
import html
import json
import os
import pathlib
import re
import shutil
import struct
import subprocess
import sys
import zlib

try:  # The fast path; the numbers are the same without it.
    import numpy
except ImportError:  # pragma: no cover - plain Python
    numpy = None

HERE = pathlib.Path(__file__).resolve().parent
SCENARIOS = HERE / "scenarios"
THRESHOLDS = SCENARIOS / "fidelity-thresholds.json"
PAGES = json.loads((HERE / "sample" / "fidelity-pages.json").read_text())
DEFAULT_CONFIGS = ["light@1", "dark@1", "light@1.25", "dark@1.25"]
# The host's scenario for each scheme and scale; a page's module and window replace the template's.
TEMPLATES = {("light", 1.0): "fidelity.json", ("light", 1.25): "fidelity-scaled.json",
             ("dark", 1.0): "fidelity-dark.json", ("dark", 1.25): "fidelity-dark-scaled.json"}
FIDELITY_LINE = re.compile(r"\[fidelity\] (\{.*\})\s*$")


# --- PNG ------------------------------------------------------------------------------------------

class Image:
    """8-bit RGB pixels, row by row."""

    def __init__(self, width, height, data):
        self.width, self.height, self.data = width, height, bytes(data)

    def crop(self, x, y, width, height):
        if x < 0 or y < 0 or x + width > self.width or y + height > self.height:
            raise ValueError(f"crop {x},{y} {width}x{height} is outside {self.width}x{self.height}")
        stride = self.width * 3
        rows = [self.data[(y + r) * stride + x * 3:(y + r) * stride + (x + width) * 3] for r in range(height)]
        return Image(width, height, b"".join(rows))


def read_png(path):
    """A PNG as RGB (8-bit greyscale, RGB or RGBA, any filter, not interlaced)."""
    data = pathlib.Path(path).read_bytes()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError(f"{path} is not a PNG")
    at, idat, header = 8, [], None
    while at < len(data):
        length, kind = struct.unpack(">I4s", data[at:at + 8])
        body = data[at + 8:at + 8 + length]
        if kind == b"IHDR":
            header = struct.unpack(">IIBBBBB", body)
        elif kind == b"IDAT":
            idat.append(body)
        elif kind == b"IEND":
            break
        at += 12 + length
    width, height, depth, colour, _, _, interlace = header
    channels = {0: 1, 2: 3, 4: 2, 6: 4}.get(colour)
    if depth != 8 or channels is None or interlace:
        raise ValueError(f"{path}: depth {depth}, colour type {colour}, interlace {interlace} not read")
    raw = zlib.decompress(b"".join(idat))
    bpp, stride = channels, width * channels
    rows, previous = [], bytearray(stride)
    for y in range(height):
        start = y * (stride + 1)
        kind, line = raw[start], bytearray(raw[start + 1:start + 1 + stride])
        if kind == 1:
            for i in range(bpp, stride):
                line[i] = (line[i] + line[i - bpp]) & 255
        elif kind == 2:
            for i in range(stride):
                line[i] = (line[i] + previous[i]) & 255
        elif kind == 3:
            for i in range(stride):
                left = line[i - bpp] if i >= bpp else 0
                line[i] = (line[i] + ((left + previous[i]) >> 1)) & 255
        elif kind == 4:
            for i in range(stride):
                a = line[i - bpp] if i >= bpp else 0
                b = previous[i]
                c = previous[i - bpp] if i >= bpp else 0
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                line[i] = (line[i] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
        rows.append(bytes(line))
        previous = line
    pixels = b"".join(rows)
    if channels == 3:
        return Image(width, height, pixels)
    out = bytearray(width * height * 3)
    if channels == 4:  # composited over nothing: the frames shot here are opaque
        out[0::3], out[1::3], out[2::3] = pixels[0::4], pixels[1::4], pixels[2::4]
    elif channels == 1:
        out[0::3] = out[1::3] = out[2::3] = pixels
    else:
        out[0::3] = out[1::3] = out[2::3] = pixels[0::2]
    return Image(width, height, out)


def write_png(path, image):
    def chunk(kind, body):
        return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body) & 0xffffffff)
    stride = image.width * 3
    raw = b"".join(b"\x00" + image.data[y * stride:(y + 1) * stride] for y in range(image.height))
    pathlib.Path(path).write_bytes(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", image.width, image.height, 8, 2, 0, 0, 0))
                                   + chunk(b"IDAT", zlib.compress(raw, 6)) + chunk(b"IEND", b""))


# --- Comparing ------------------------------------------------------------------------------------

C1, C2 = (0.01 * 255) ** 2, (0.03 * 255) ** 2
WINDOW, STRIDE = 8, 4


def windows(length):
    """Where the windows along a side start (one window over a side shorter than a window)."""
    if length <= WINDOW:
        return [0], length
    return list(range(0, length - WINDOW + 1, STRIDE)), WINDOW


def ssim_from_sums(n, sx, sy, sxx, syy, sxy):
    mx, my = sx / n, sy / n
    vx, vy, cov = sxx / n - mx * mx, syy / n - my * my, sxy / n - mx * my
    return ((2 * mx * my + C1) * (2 * cov + C2)) / ((mx * mx + my * my + C1) * (vx + vy + C2))


def compare(a, b, tolerance):
    """mae, over (percent), ssim and the largest difference of two crops of one size."""
    assert (a.width, a.height) == (b.width, b.height)
    if numpy is not None:
        return compare_numpy(a, b, tolerance)
    w, h = a.width, a.height
    da, db = a.data, b.data
    total = over = largest = 0
    luma_a, luma_b = [0.0] * (w * h), [0.0] * (w * h)
    for i in range(w * h):
        ra, ga, ba = da[3 * i], da[3 * i + 1], da[3 * i + 2]
        rb, gb, bb = db[3 * i], db[3 * i + 1], db[3 * i + 2]
        d = max(abs(ra - rb), abs(ga - gb), abs(ba - bb))
        total += abs(ra - rb) + abs(ga - gb) + abs(ba - bb)
        if d > tolerance:
            over += 1
        if d > largest:
            largest = d
        luma_a[i] = 0.299 * ra + 0.587 * ga + 0.114 * ba
        luma_b[i] = 0.299 * rb + 0.587 * gb + 0.114 * bb
    # Integral images of x, y, x², y² and xy.
    W = w + 1
    sums = [[0.0] * (W * (h + 1)) for _ in range(5)]
    sx, sy, sxx, syy, sxy = sums
    for y in range(h):
        rx = ry = rxx = ryy = rxy = 0.0
        row, above, base = (y + 1) * W, y * W, y * w
        for x in range(w):
            p, q = luma_a[base + x], luma_b[base + x]
            rx += p
            ry += q
            rxx += p * p
            ryy += q * q
            rxy += p * q
            k, j = row + x + 1, above + x + 1
            sx[k] = sx[j] + rx
            sy[k] = sy[j] + ry
            sxx[k] = sxx[j] + rxx
            syy[k] = syy[j] + ryy
            sxy[k] = sxy[j] + rxy
    xs, ww = windows(w)
    ys, wh = windows(h)
    n, values = ww * wh, []
    for y0 in ys:
        for x0 in xs:
            def box(s):
                return s[(y0 + wh) * W + x0 + ww] - s[y0 * W + x0 + ww] - s[(y0 + wh) * W + x0] + s[y0 * W + x0]
            values.append(ssim_from_sums(n, box(sx), box(sy), box(sxx), box(syy), box(sxy)))
    return {"mae": total / (3 * w * h), "over": 100.0 * over / (w * h), "ssim": sum(values) / len(values),
            "largest": largest}


def compare_numpy(a, b, tolerance):
    w, h = a.width, a.height
    pa = numpy.frombuffer(a.data, numpy.uint8).reshape(h, w, 3).astype(numpy.int32)
    pb = numpy.frombuffer(b.data, numpy.uint8).reshape(h, w, 3).astype(numpy.int32)
    diff = numpy.abs(pa - pb)
    per_pixel = diff.max(axis=2)
    weights = numpy.array([0.299, 0.587, 0.114])
    la, lb = pa @ weights, pb @ weights

    def integral(values):
        out = numpy.zeros((h + 1, w + 1))
        out[1:, 1:] = values.cumsum(axis=1).cumsum(axis=0)
        return out
    sums = [integral(v) for v in (la, lb, la * la, lb * lb, la * lb)]
    xs, ww = windows(w)
    ys, wh = windows(h)
    y0 = numpy.array(ys)[:, None]
    x0 = numpy.array(xs)[None, :]

    def box(s):
        return s[y0 + wh, x0 + ww] - s[y0, x0 + ww] - s[y0 + wh, x0] + s[y0, x0]
    values = ssim_from_sums(ww * wh, *(box(s) for s in sums))
    return {"mae": float(diff.sum()) / (3 * w * h), "over": 100.0 * float((per_pixel > tolerance).sum()) / (w * h),
            "ssim": float(values.mean()), "largest": int(per_pixel.max())}


def heatmap(a, b):
    """Where the crops differ: the host's crop greyed and darkened, each difference over it from
    red (a few levels) to yellow and white (far apart)."""
    if numpy is not None:
        return heatmap_numpy(a, b)
    out = bytearray(a.width * a.height * 3)
    da, db = a.data, b.data
    for i in range(a.width * a.height):
        d = max(abs(da[3 * i] - db[3 * i]), abs(da[3 * i + 1] - db[3 * i + 1]), abs(da[3 * i + 2] - db[3 * i + 2]))
        grey = int((0.299 * db[3 * i] + 0.587 * db[3 * i + 1] + 0.114 * db[3 * i + 2]) * 0.25)
        if d <= 2:
            out[3 * i:3 * i + 3] = bytes((grey, grey, grey))
        else:
            t = min(1.0, d / 96.0)
            out[3 * i:3 * i + 3] = bytes((255, int(255 * max(0.0, (t - 0.35) / 0.65)),
                                          int(255 * max(0.0, (t - 0.8) / 0.2))))
    return Image(a.width, a.height, out)


def heatmap_numpy(a, b):
    w, h = a.width, a.height
    pa = numpy.frombuffer(a.data, numpy.uint8).reshape(h, w, 3).astype(numpy.int32)
    pb = numpy.frombuffer(b.data, numpy.uint8).reshape(h, w, 3).astype(numpy.int32)
    d = numpy.abs(pa - pb).max(axis=2)
    grey = ((pb @ numpy.array([0.299, 0.587, 0.114])) * 0.25).astype(numpy.int32)
    t = numpy.minimum(1.0, d / 96.0)
    out = numpy.stack([numpy.full_like(grey, 255), (255 * numpy.maximum(0.0, (t - 0.35) / 0.65)).astype(numpy.int32),
                       (255 * numpy.maximum(0.0, (t - 0.8) / 0.2)).astype(numpy.int32)], axis=2)
    out[d <= 2] = numpy.stack([grey, grey, grey], axis=2)[d <= 2]
    return Image(w, h, out.astype(numpy.uint8).tobytes())


# --- Running both sides ---------------------------------------------------------------------------

def parse_configs(text):
    configs = []
    for item in text.split(","):
        scheme, _, scale = item.strip().partition("@")
        if scheme not in ("light", "dark") or not scale:
            raise SystemExit(f"a configuration is SCHEME@SCALE (light@1, dark@1.25): {item!r}")
        configs.append((scheme, float(scale)))
    return configs


def config_name(scheme, scale):
    return f"{scheme}@{scale:g}"


def pages_of(text):
    known = {page["page"]: page for page in PAGES}
    wanted = [int(p) for p in text.split(",")] if text else sorted(known)
    for p in wanted:
        if p not in known:
            raise SystemExit(f"no page {p} (fidelity/pages.json has {sorted(known)})")
    return [known[p] for p in wanted]


def bloom_dir(option):
    bloom = option or os.environ.get("BLOOM_DIR")
    if not bloom:
        raise SystemExit("--bloom (or BLOOM_DIR): the Bloom checkout")
    return str(pathlib.Path(bloom).resolve())


def run_web(out, bloom, pages, configs):
    """The web export and Chromium's frames: OUT/web/page<N>-<config>.png and .json."""
    fidelity = HERE / "fidelity"
    if not (fidelity / "node_modules" / "playwright-core").exists():
        raise SystemExit(f"playwright-core is missing: run `npm ci` in {fidelity}")
    build = out / "web-build"
    env = dict(os.environ, BLOOM_DIR=bloom)
    log = out / "web-build.log"
    with open(log, "w") as stream:
        done = subprocess.run(["node", str(fidelity / "web.mjs"), str(build)], env=env, stdout=stream,
                              stderr=subprocess.STDOUT)
    if done.returncode != 0:
        raise SystemExit(f"the web build failed; see {log}")
    shots = out / "web"
    shots.mkdir(parents=True, exist_ok=True)
    spec = {"dist": str(build / "dist"), "out": str(shots),
            "pages": pages, "configs": [{"scheme": s, "scale": k, "name": config_name(s, k)} for s, k in configs]}
    done = subprocess.run(["node", str(fidelity / "shoot.mjs"), json.dumps(spec)])
    if done.returncode != 0:
        raise SystemExit("shooting the web pages failed")


def host_scenario(out, page, scheme, scale):
    """The page's scenario: the template for its scheme and scale, its module and window."""
    template = json.loads((SCENARIOS / TEMPLATES[(scheme, scale)]).read_text())
    template["module"] = "FidelityGallery" if page["page"] == 1 else f"FidelityGallery{page['page']}"
    template["window"] = [page["width"], page["height"]]
    for step in template["steps"]:
        texts = step.get("expect", {}).get("texts")
        if texts:
            step["expect"]["texts"] = [t.replace("page 1", f"page {page['page']}") for t in texts]
    path = out / "scenarios" / f"page{page['page']}-{config_name(scheme, scale)}.json"
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(template, indent=2))
    return path


def run_host_one(out, options, page, scheme, scale):
    name = f"page{page['page']}-{config_name(scheme, scale)}"
    scenario = host_scenario(out, page, scheme, scale)
    run = out / "host" / name
    shutil.rmtree(run, ignore_errors=True)
    done = subprocess.run([sys.executable, str(HERE / "regress.py"), str(scenario), "--offscreen", options.offscreen,
                           "--runtimes", options.runtimes, "--packages", options.packages, "--out", str(run)],
                          capture_output=True, text=True)
    (out / "host" / f"{name}.txt").write_text(done.stdout + done.stderr)
    log = run / "director.log"
    if not log.exists():
        return name, f"regress.py wrote no director.log (status {done.returncode})"
    text = log.read_text(errors="replace")
    summary = next((json.loads(line) for line in text.splitlines() if line.startswith('{"mode"')), None)
    layouts = [json.loads(m.group(1)) for m in map(FIDELITY_LINE.search, text.splitlines()) if m]
    steps = json.loads((scenario).read_text())["steps"]
    shot = run / f"launch-{len(steps) + 1}.png"
    if summary is None or not layouts or not shot.exists():
        return name, f"the host drew no gallery (status {done.returncode}); see {log}"
    window = (summary["steps"][-1] if summary["steps"] else summary)["window"]
    layout = layouts[-1]
    layout["window"] = window
    layout["frame"] = str(shot)
    layout["regress"] = done.returncode
    (out / "host" / f"{name}.json").write_text(json.dumps(layout, indent=1))
    return name, None if done.returncode == 0 else f"regress.py failed ({done.returncode}); see {out / 'host' / (name + '.txt')}"


def run_host(out, options, pages, configs):
    (out / "host").mkdir(parents=True, exist_ok=True)
    jobs = [(page, scheme, scale) for page in pages for scheme, scale in configs]
    problems = []
    with concurrent.futures.ThreadPoolExecutor(max_workers=max(1, options.jobs)) as pool:
        for name, problem in pool.map(lambda job: run_host_one(out, options, *job), jobs):
            print(f"host {name}: {problem or 'ok'}", file=sys.stderr)
            if problem:
                problems.append(f"{name}: {problem}")
    return problems


# --- The report -----------------------------------------------------------------------------------

def thresholds_for(thresholds, cell):
    merged = dict(thresholds.get("default", {}))
    merged.update(thresholds.get("cells", {}).get(cell, {}))
    return merged


def crop_rect(layout, cell, scale, origin):
    x, y, w, h = (layout["cells"][cell][k] for k in ("x", "y", "w", "h"))
    return (origin[0] + round(x * scale), origin[1] + round(y * scale), round(w * scale), round(h * scale))


def report(out, check=False):
    thresholds = json.loads(THRESHOLDS.read_text())
    tolerance = thresholds.get("tolerance", 16)
    expected = thresholds.get("expected", {})
    crops = out / "crops"
    shutil.rmtree(crops, ignore_errors=True)
    results, layout_diffs, missing = [], [], []
    for page in PAGES:
        for (scheme, scale) in [(s, k) for s in ("light", "dark") for k in (1.0, 1.25)]:
            config = config_name(scheme, scale)
            name = f"page{page['page']}-{config}"
            web_json, host_json = out / "web" / f"{name}.json", out / "host" / f"{name}.json"
            if not web_json.exists() and not host_json.exists():
                continue
            if not web_json.exists() or not host_json.exists():
                missing.append({"page": page["page"], "config": config, "side": "web" if not web_json.exists() else "host",
                                "cell": "*"})
                continue
            web, host = json.loads(web_json.read_text()), json.loads(host_json.read_text())
            web_frame = read_png(out / "web" / f"{name}.png")
            host_frame = read_png(host["frame"])
            # The host's window in the output (the director gives it in the output's logical
            # pixels), in the frame's pixels.
            origin = (round(host["window"][0] * scale), round(host["window"][1] * scale))
            folder = crops / config
            folder.mkdir(parents=True, exist_ok=True)
            for cell in sorted(set(web["cells"]) | set(host["cells"])):
                if cell not in web["cells"] or cell not in host["cells"]:
                    missing.append({"page": page["page"], "config": config, "cell": cell,
                                    "side": "web" if cell not in web["cells"] else "host"})
                    continue
                wc, hc = web["cells"][cell], host["cells"][cell]
                rect_web = crop_rect(web, cell, scale, (0, 0))
                rect_host = crop_rect(host, cell, scale, origin)
                if (wc["x"], wc["y"], wc["w"], wc["h"]) != (hc["x"], hc["y"], hc["w"], hc["h"]):
                    layout_diffs.append({"cell": cell, "config": config, "kind": "cell",
                                         "web": [wc[k] for k in "xywh"], "host": [hc[k] for k in "xywh"]})
                if abs(wc["cw"] - hc["cw"]) > 0.5 or abs(wc["ch"] - hc["ch"]) > 0.5:
                    layout_diffs.append({"cell": cell, "config": config, "kind": "content",
                                         "web": [round(wc["cw"], 2), round(wc["ch"], 2)],
                                         "host": [round(hc["cw"], 2), round(hc["ch"], 2)]})
                a = web_frame.crop(*rect_web)
                b = host_frame.crop(rect_host[0], rect_host[1], rect_web[2], rect_web[3])
                scores = compare(a, b, tolerance)
                stem = cell.replace("/", "_")
                write_png(folder / f"{stem}.web.png", a)
                write_png(folder / f"{stem}.host.png", b)
                write_png(folder / f"{stem}.diff.png", heatmap(a, b))
                limit = thresholds_for(thresholds, cell)
                failed = []
                if scores["ssim"] < limit.get("ssim_min", 0):
                    failed.append(f"ssim {scores['ssim']:.3f} < {limit['ssim_min']}")
                if scores["over"] > limit.get("over_max", 100):
                    failed.append(f"over {scores['over']:.1f}% > {limit['over_max']}%")
                if scores["mae"] > limit.get("mae_max", 255):
                    failed.append(f"mae {scores['mae']:.1f} > {limit['mae_max']}")
                results.append({"cell": cell, "page": page["page"], "config": config, "scheme": scheme, "scale": scale,
                                **{k: round(v, 4) if isinstance(v, float) else v for k, v in scores.items()},
                                "label": web["cells"][cell].get("label", cell),
                                "expected": expected.get(cell), "failed": failed if not expected.get(cell) else [],
                                "crops": {side: str((folder / f"{stem}.{side}.png").relative_to(out))
                                          for side in ("web", "host", "diff")}})
    # Worst first; the cells expected to differ after every other.
    results.sort(key=lambda r: (bool(r["expected"]), r["ssim"], -r["over"]))
    scores_path = out / "scores.json"
    scores_path.write_text(json.dumps({"tolerance": tolerance, "cells": results, "layout": layout_diffs,
                                       "missing": missing, "numpy": numpy is not None}, indent=1))
    write_html(out, results, layout_diffs, missing, thresholds)
    failing = [r for r in results if r["failed"]]
    print(f"{len(results)} cells compared; report: {out / 'report.html'}; scores: {scores_path}", file=sys.stderr)
    for r in [r for r in results if not r["expected"]][:20]:
        print(f"  {r['ssim']:.3f} ssim  {r['mae']:6.2f} mae  {r['over']:5.1f}% over  {r['config']:9} {r['cell']}",
              file=sys.stderr)
    if check:
        for r in failing:
            print(f"FAIL {r['config']} {r['cell']}: {'; '.join(r['failed'])}", file=sys.stderr)
        for m in missing:
            print(f"FAIL {m['config']} page {m['page']} {m['cell']}: missing on the {m['side']}", file=sys.stderr)
        if not results:
            print("FAIL: nothing compared", file=sys.stderr)
        return 1 if failing or missing or not results else 0
    return 0


def write_html(out, results, layout_diffs, missing, thresholds):
    def row(rank, r):
        img = lambda side: f'<img loading="lazy" src="{html.escape(r["crops"][side])}" alt="{side}">'
        flag = f'<div class="flag">expected to differ: {html.escape(r["expected"])}</div>' if r["expected"] else ""
        fail = f'<div class="fail">{html.escape("; ".join(r["failed"]))}</div>' if r["failed"] else ""
        return (f'<tr><td class="n">{rank}</td><td><b>{html.escape(r["cell"])}</b><div class="dim">{html.escape(r["label"])}'
                f' · page {r["page"]} · {r["config"]}</div>{flag}{fail}</td>'
                f'<td class="n">{r["ssim"]:.3f}</td><td class="n">{r["mae"]:.2f}</td><td class="n">{r["over"]:.1f}%</td>'
                f'<td class="n">{r["largest"]}</td><td class="pics">{img("web")}{img("host")}{img("diff")}</td></tr>')
    head = ('<tr><th>#</th><th>cell</th><th>SSIM</th><th>MAE</th><th>over</th><th>max</th>'
            '<th>web (Chromium) · host · difference</th></tr>')
    configs = sorted({r["config"] for r in results}, key=lambda c: (c.split("@")[0], float(c.split("@")[1])))
    expected = [r for r in results if r["expected"]]
    results = [r for r in results if not r["expected"]]
    summary = "".join(
        f'<tr><td>{c}</td><td class="n">{len([r for r in results if r["config"] == c])}</td>'
        f'<td class="n">{sum(r["ssim"] for r in results if r["config"] == c) / max(1, len([r for r in results if r["config"] == c])):.3f}</td>'
        f'<td class="n">{sum(r["mae"] for r in results if r["config"] == c) / max(1, len([r for r in results if r["config"] == c])):.2f}</td>'
        f'<td class="n">{len([r for r in results if r["config"] == c and r["failed"]])}</td></tr>' for c in configs)
    sections = [f'<h2 id="worst">Worst 40, every configuration</h2><table>{head}'
                + "".join(row(i + 1, r) for i, r in enumerate(results[:40])) + "</table>"]
    # Each cell's worst configuration, and its mean over all of them.
    cells = {}
    for r in results:
        cells.setdefault(r["cell"], []).append(r)
    per_cell = sorted(cells.items(), key=lambda item: min(r["ssim"] for r in item[1]))
    sections.append('<h2 id="cells">By cell (worst configuration first)</h2><table><tr><th>#</th><th>cell</th>'
                    '<th>worst SSIM</th><th>in</th><th>mean SSIM</th><th>mean MAE</th><th>mean over</th></tr>' + "".join(
                        f'<tr><td class="n">{i + 1}</td><td>{html.escape(cell)}</td>'
                        f'<td class="n">{min(r["ssim"] for r in rows):.3f}</td><td>{min(rows, key=lambda r: r["ssim"])["config"]}</td>'
                        f'<td class="n">{sum(r["ssim"] for r in rows) / len(rows):.3f}</td>'
                        f'<td class="n">{sum(r["mae"] for r in rows) / len(rows):.2f}</td>'
                        f'<td class="n">{sum(r["over"] for r in rows) / len(rows):.1f}%</td></tr>'
                        for i, (cell, rows) in enumerate(per_cell)) + '</table>')
    for c in configs:
        rows = [r for r in results if r["config"] == c]
        sections.append(f'<h2 id="{c}">{c}</h2><table>{head}' + "".join(row(i + 1, r) for i, r in enumerate(rows)) + "</table>")
    if expected:
        sections.append(f'<h2 id="expected">Expected to differ (not ranked)</h2><table>{head}'
                        + "".join(row(i + 1, r) for i, r in enumerate(expected)) + "</table>")
    # One row per cell and difference, the configurations it was seen in together.
    grouped = {}
    for d in layout_diffs:
        grouped.setdefault((d["cell"], d["kind"], str(d["web"]), str(d["host"])), []).append(d["config"])
    layout = "".join(f'<tr><td>{html.escape(cell)}</td><td>{", ".join(seen)}</td><td>{kind}</td>'
                     f'<td>{web}</td><td>{host}</td></tr>' for (cell, kind, web, host), seen in sorted(grouped.items()))
    gone = "".join(f'<li>page {m["page"]} {m["config"]} {html.escape(m["cell"])}: missing on the {m["side"]}</li>' for m in missing)
    nav = " · ".join(f'<a href="#{c}">{c}</a>' for c in configs)
    target = thresholds.get("target", {})
    page = f"""<!doctype html><html lang="en"><head><meta charset="utf-8"><title>Bloom fidelity</title>
<meta name="viewport" content="width=device-width, initial-scale=1">
<style>
:root {{ --bg: #f6f6f4; --fg: #1d1d1f; --dim: #6e6e73; --line: #d8d8d6; --card: #ffffff; --bad: #c62828; --warn: #8a6d00; }}
@media (prefers-color-scheme: dark) {{ :root {{ --bg: #161618; --fg: #ececee; --dim: #9a9aa0; --line: #333336; --card: #1f1f22; --bad: #ff6b6b; --warn: #e0b84a; }} }}
body {{ background: var(--bg); color: var(--fg); font: 14px/1.4 system-ui, sans-serif; margin: 0 auto; padding: 16px; max-width: 1800px; }}
table {{ border-collapse: collapse; width: 100%; background: var(--card); margin-bottom: 24px; }}
td, th {{ border-bottom: 1px solid var(--line); padding: 6px 8px; vertical-align: top; text-align: left; }}
td.n {{ text-align: right; font-variant-numeric: tabular-nums; white-space: nowrap; }}
.dim {{ color: var(--dim); font-size: 12px; }} .flag {{ color: var(--warn); font-size: 12px; }} .fail {{ color: var(--bad); font-size: 12px; }}
.pics {{ display: flex; gap: 6px; flex-wrap: wrap; }} .pics img {{ max-width: 100%; image-rendering: pixelated; border: 1px solid var(--line); }}
a {{ color: inherit; }}
</style></head><body>
<h1>Bloom fidelity: Chromium (react-native-web) against the OxyOS host</h1>
<p class="dim">{len(results)} cells compared and ranked, {len(expected)} expected to differ ·
generated {datetime.datetime.now().isoformat(timespec="seconds")} · <a href="#cells">by cell</a> ·
SSIM on luminance (8×8 windows, stride 4) · over = pixels with a channel more than {thresholds.get("tolerance", 16)} levels apart ·
target: SSIM ≥ {target.get("ssim_min", "?")}, over ≤ {target.get("over_max", "?")}% · {nav}</p>
<table><tr><th>configuration</th><th>cells</th><th>mean SSIM</th><th>mean MAE</th><th>over threshold</th></tr>{summary}</table>
<h2>Layout (measurement, not painting)</h2>
<p class="dim">A cell's content laid out at another size on the host than in the browser (logical pixels, w×h).</p>
<table><tr><th>cell</th><th>config</th><th>what</th><th>web</th><th>host</th></tr>{layout or '<tr><td colspan=5>none</td></tr>'}</table>
{'<h2>Missing</h2><ul>' + gone + '</ul>' if gone else ''}
{''.join(sections)}
</body></html>"""
    (out / "report.html").write_text(page)


def baseline(out):
    scores = json.loads((out / "scores.json").read_text())
    thresholds = json.loads(THRESHOLDS.read_text())
    by_cell = {}
    for r in scores["cells"]:
        by_cell.setdefault(r["cell"], []).append(r)
    cells = thresholds.setdefault("cells", {})
    recorded = {}
    for cell, rows in sorted(by_cell.items()):
        if rows[0].get("expected"):
            continue
        # A floor below today's worst of every scheme and scale: drawing it worse fails.
        cells[cell] = {"status": "baseline",
                       "ssim_min": max(0.0, round(min(r["ssim"] for r in rows) - 0.02, 3)),
                       "over_max": min(100.0, round(max(r["over"] for r in rows) + 2.0, 1)),
                       "mae_max": round(max(r["mae"] for r in rows) * 1.1 + 1.0, 1)}
        # ssim, mae and over for each configuration, in the order of DEFAULT_CONFIGS.
        recorded[cell] = {r["config"]: [r["ssim"], round(r["mae"], 2), round(r["over"], 2)] for r in rows}
    commit = subprocess.run(["git", "-C", str(HERE), "rev-parse", "--short", "HEAD"], capture_output=True, text=True).stdout.strip()
    thresholds["baseline"] = {"host": commit, "date": datetime.date.today().isoformat(),
                              "columns": ["ssim", "mae", "over"], "scores": recorded}
    write_thresholds(thresholds)
    print(f"recorded {len(cells)} cells' floors in {THRESHOLDS}", file=sys.stderr)


def write_thresholds(thresholds):
    """The thresholds file, a cell to a line, so a change of floors reads as a diff of cells."""
    def block(entries, indent):
        return "{\n" + ",\n".join(f'{indent}{json.dumps(key)}: {json.dumps(value, ensure_ascii=False)}'
                                  for key, value in entries.items()) + "\n" + indent[:-2] + "}"
    parts = []
    for key, value in thresholds.items():
        if key in ("cells", "expected"):
            parts.append(f'  {json.dumps(key)}: {block(value, "    ") if value else "{}"}')
        elif key == "baseline":
            head = {k: v for k, v in value.items() if k != "scores"}
            inner = [f'    {json.dumps(k)}: {json.dumps(v)}' for k, v in head.items()]
            inner.append(f'    "scores": {block(value["scores"], "      ")}')
            parts.append('  "baseline": {\n' + ",\n".join(inner) + "\n  }")
        else:
            parts.append(f'  {json.dumps(key)}: {json.dumps(value, ensure_ascii=False)}')
    THRESHOLDS.write_text("{\n" + ",\n".join(parts) + "\n}\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = parser.add_subparsers(dest="command", required=True)
    for name in ("run", "web", "host", "report", "baseline"):
        p = sub.add_parser(name)
        p.add_argument("--out", required=True)
        if name in ("run", "web", "host"):
            p.add_argument("--pages", default=None, help="pages to shoot (1,2,...; all by default)")
            p.add_argument("--configs", default=",".join(DEFAULT_CONFIGS))
        if name in ("run", "web"):
            p.add_argument("--bloom", default=None, help="the Bloom checkout (BLOOM_DIR)")
        if name in ("run", "host"):
            p.add_argument("--offscreen", required=True)
            p.add_argument("--runtimes", required=True)
            p.add_argument("--packages", required=True)
            p.add_argument("--jobs", type=int, default=4)
        if name in ("run", "report"):
            p.add_argument("--check", action="store_true", help="exit 1 when a cell passes its threshold")
    options = parser.parse_args()
    out = pathlib.Path(options.out).resolve()
    out.mkdir(parents=True, exist_ok=True)
    if options.command in ("run", "web", "host"):
        pages, configs = pages_of(options.pages), parse_configs(options.configs)
    if options.command in ("run", "web"):
        run_web(out, bloom_dir(options.bloom), pages, configs)
    problems = []
    if options.command in ("run", "host"):
        problems = run_host(out, options, pages, configs)
        for problem in problems:
            print(f"host: {problem}", file=sys.stderr)
    if options.command in ("run", "report"):
        status = report(out, check=options.check)
        return status or (1 if options.check and problems else 0)
    if options.command == "baseline":
        baseline(out)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
