//! Renders Solar Gradients, OxyOS's own procedural dynamic wallpaper (docs/adr/0048-wallpapers.mdx),
//! from `scripts/solar-gradients.json`: each variant a sky of two radial gradients whose colour
//! stops are OKLab, one centred above the picture and one, flattened, below its bottom edge,
//! summed in OKLab, then turned into sRGB and dithered (triangular noise of one level, from a hash
//! of the pixel) so that what is shallower than a level a pixel never bands. Writes each variant's
//! JPEG and the catalog's `wallpaper.json`:
//!
//! ```text
//! cargo run --release -p oxyos-wallpaper --example make-solar-gradients -- \
//!     scripts/solar-gradients.json config/includes.chroot_after_packages/usr/share/oxyos/wallpapers/solar-gradients
//! ```
//!
//! `--size WxH` renders at another size (a preview); the default is 6144 × 3456, 6K at 16:9.

use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize)]
struct Params {
    geometry: Geometry,
    variants: Vec<Variant>,
}

#[derive(Deserialize)]
struct Geometry {
    sky: Radial,
    glow: Radial,
}

/// A radial gradient: its centre (across and down, in the picture's widths and heights), how much
/// wider than tall its circles are, and the distances (in heights) its first and last stops are at.
#[derive(Deserialize)]
struct Radial {
    centre: [f64; 2],
    #[serde(default = "one")]
    stretch: f64,
    from: f64,
    to: f64,
}

fn one() -> f64 {
    1.0
}

impl Radial {
    /// Where (x across from the middle, y down, in heights) is along its stops, 0 to 1.
    fn at(&self, x: f64, y: f64, aspect: f64) -> f64 {
        let cx = (self.centre[0] - 0.5) * aspect;
        let d = ((x - cx) / self.stretch).hypot(y - self.centre[1]);
        (d - self.from) / (self.to - self.from)
    }
}

#[derive(Deserialize)]
struct Names {
    en: String,
    es: String,
}

#[derive(Deserialize)]
struct Sun {
    elevation: f64,
    azimuth: f64,
}

#[derive(Deserialize)]
struct Variant {
    id: String,
    name: Names,
    sun: Sun,
    sky: Vec<[f64; 3]>,
    glow: Vec<[f64; 3]>,
}

/// The stops' curve at `t` in [0, 1]: Catmull-Rom through evenly spaced stops, its ends held.
fn curve(stops: &[[f64; 3]], t: f64) -> [f64; 3] {
    let n = stops.len();
    let u = t.clamp(0.0, 1.0) * (n - 1) as f64;
    let i = (u.floor() as usize).min(n - 2);
    let f = u - i as f64;
    let at = |k: isize| stops[k.clamp(0, n as isize - 1) as usize];
    let (p0, p1, p2, p3) = (
        at(i as isize - 1),
        at(i as isize),
        at(i as isize + 1),
        at(i as isize + 2),
    );
    let mut out = [0.0; 3];
    for c in 0..3 {
        out[c] = 0.5
            * (2.0 * p1[c]
                + (-p0[c] + p2[c]) * f
                + (2.0 * p0[c] - 5.0 * p1[c] + 4.0 * p2[c] - p3[c]) * f * f
                + (-p0[c] + 3.0 * p1[c] - 3.0 * p2[c] + p3[c]) * f * f * f);
    }
    out
}

/// OKLab to linear sRGB (Björn Ottosson's).
fn oklab_to_linear([l, a, b]: [f64; 3]) -> [f64; 3] {
    let l_ = l + 0.396_337_777_4 * a + 0.215_803_757_3 * b;
    let m_ = l - 0.105_561_345_8 * a - 0.063_854_172_8 * b;
    let s_ = l - 0.089_484_177_5 * a - 1.291_485_548 * b;
    let (l3, m3, s3) = (l_ * l_ * l_, m_ * m_ * m_, s_ * s_ * s_);
    [
        4.076_741_662_1 * l3 - 3.307_711_591_3 * m3 + 0.230_969_929_2 * s3,
        -1.268_438_004_6 * l3 + 2.609_757_401_1 * m3 - 0.341_319_396_5 * s3,
        -0.004_196_086_3 * l3 - 0.703_418_614_7 * m3 + 1.707_614_701 * s3,
    ]
}

fn encode_srgb(c: f64) -> f64 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.003_130_8 {
        12.92 * c
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// A uniform number in [0, 1) from a pixel and a seed: SplitMix64's finalizer.
fn uniform(x: u32, y: u32, seed: u64) -> f64 {
    let mut h = (u64::from(x) << 32 | u64::from(y)) ^ seed.wrapping_mul(0x9e37_79b9_7f4a_7c15);
    h ^= h >> 30;
    h = h.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    h ^= h >> 27;
    h = h.wrapping_mul(0x94d0_49bb_1331_11eb);
    h ^= h >> 31;
    (h >> 11) as f64 / (1u64 << 53) as f64
}

fn render(geometry: &Geometry, variant: &Variant, seed: u64, width: u32, height: u32) -> Vec<u8> {
    let aspect = f64::from(width) / f64::from(height);
    let mut rgb = Vec::with_capacity(width as usize * height as usize * 3);
    for py in 0..height {
        let y = (f64::from(py) + 0.5) / f64::from(height);
        for px in 0..width {
            // In heights from the picture's middle, across a 16:9 sky (a wider or narrower
            // picture shows more or less of it at its sides).
            let x = ((f64::from(px) + 0.5) / f64::from(width) - 0.5) * aspect;
            let sky = curve(&variant.sky, geometry.sky.at(x, y, aspect));
            let glow = curve(&variant.glow, geometry.glow.at(x, y, aspect));
            let linear = oklab_to_linear([sky[0] + glow[0], sky[1] + glow[1], sky[2] + glow[2]]);
            for (c, value) in linear.into_iter().enumerate() {
                let level = encode_srgb(value) * 255.0;
                // Triangular dither of one level, the channels' own.
                let stream = seed * 3 + c as u64;
                let noise = uniform(px, py, stream) + uniform(px, py, stream | 1 << 40) - 1.0;
                rgb.push((level + noise).round().clamp(0.0, 255.0) as u8);
            }
        }
    }
    rgb
}

fn main() {
    let mut args = std::env::args().skip(1);
    let params_path = PathBuf::from(args.next().expect("the parameters' JSON"));
    let out = PathBuf::from(args.next().expect("the wallpaper's folder"));
    let (mut width, mut height) = (6144u32, 3456u32);
    let mut only: Option<String> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--size" => {
                let size = args.next().expect("WxH");
                let (w, h) = size.split_once('x').expect("WxH");
                (width, height) = (w.parse().expect("a width"), h.parse().expect("a height"));
            }
            "--only" => only = args.next(),
            other => panic!("unknown argument {other}"),
        }
    }
    let params: Params =
        serde_json::from_str(&std::fs::read_to_string(&params_path).expect("readable"))
            .expect("valid");
    std::fs::create_dir_all(&out).expect("the folder");
    for (index, variant) in params.variants.iter().enumerate() {
        if only.as_ref().is_some_and(|id| *id != variant.id) {
            continue;
        }
        let rgb = render(&params.geometry, variant, index as u64 + 1, width, height);
        let path = out.join(format!("{}.jpg", variant.id));
        let mut encoder = jpeg_encoder::Encoder::new_file(&path, 92).expect("the file");
        encoder.set_sampling_factor(jpeg_encoder::SamplingFactor::R_4_4_4);
        encoder.set_progressive(false);
        encoder
            .encode(
                &rgb,
                width as u16,
                height as u16,
                jpeg_encoder::ColorType::Rgb,
            )
            .expect("encoded");
        eprintln!("{} {}x{}", path.display(), width, height);
    }
    if only.is_some() {
        return;
    }
    let variants: Vec<serde_json::Value> = params
        .variants
        .iter()
        .map(|v| {
            serde_json::json!({
                "id": v.id,
                "name": { "en": v.name.en, "es": v.name.es },
                "file": format!("{}.jpg", v.id),
                "sun": { "elevation": v.sun.elevation, "azimuth": v.sun.azimuth },
            })
        })
        .collect();
    let manifest = serde_json::json!({
        "name": { "en": "Solar Gradients", "es": "Solar Gradients" },
        "category": "dynamic",
        "automatic": "sun",
        "preview": "midday",
        "variants": variants,
    });
    std::fs::write(
        out.join("wallpaper.json"),
        serde_json::to_string_pretty(&manifest).expect("JSON") + "\n",
    )
    .expect("written");
}
