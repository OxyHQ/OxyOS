//! Thumbnails of the person's photos for Settings (docs/adr/0048-wallpapers.mdx, decision 7):
//! each decoded once at the thumbnail's pixel size (a JPEG reduced from its coefficients, as the
//! hosts decode images), cropped to it as `cover` draws it, and kept as a PNG in
//! `$XDG_CACHE_HOME/oxyos/wallpapers/thumbnails`, named by a hash of its path, size, time and the
//! thumbnail's size, so a changed photo has a new one. At most [`MAX_KEPT`] are kept, the oldest
//! going first. An SVG needs none: it is drawn at the size it is shown.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// The most thumbnails kept.
pub const MAX_KEPT: usize = 512;
/// The largest thumbnail side, in pixels.
pub const MAX_SIDE: u32 = 1024;

/// The cache's folder: `$XDG_CACHE_HOME/oxyos/wallpapers/thumbnails`, else under `~/.cache`.
pub fn directory() -> Option<PathBuf> {
    let cache = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))?;
    Some(cache.join("oxyos").join("wallpapers").join("thumbnails"))
}

/// The thumbnail of `image`, `width` × `height` pixels, in `cache`: made now unless it is there.
/// An SVG is its own.
pub fn thumbnail(image: &Path, width: u32, height: u32, cache: &Path) -> Result<PathBuf, String> {
    if !(1..=MAX_SIDE).contains(&width) || !(1..=MAX_SIDE).contains(&height) {
        return Err("a thumbnail's size is 1 to 1024 pixels a side".to_owned());
    }
    let metadata = std::fs::metadata(image).map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err("not a file".to_owned());
    }
    if image
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("svg"))
    {
        return Ok(image.to_owned());
    }
    let modified = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos());
    use std::os::unix::ffi::OsStrExt;
    let name = format!(
        "{:016x}.png",
        super::hash(&[
            image.as_os_str().as_bytes(),
            &metadata.len().to_le_bytes(),
            &modified.to_le_bytes(),
            &width.to_le_bytes(),
            &height.to_le_bytes(),
        ])
    );
    let path = cache.join(&name);
    if path.is_file() {
        // Used now: the oldest go first.
        let _ = std::fs::File::options()
            .append(true)
            .open(&path)
            .and_then(|file| file.set_modified(std::time::SystemTime::now()));
        return Ok(path);
    }
    let decoded = oxyos_image::decode_file(image, oxyos_image::Fit::Covering(width, height))
        .map_err(|error| format!("{error:?}"))?;
    let (pixels, w, h) = crop(&decoded, width, height);
    std::fs::create_dir_all(cache).map_err(|error| error.to_string())?;
    let temporary = cache.join(format!(".{}-{name}", std::process::id()));
    write_png(&temporary, &pixels, w, h)?;
    std::fs::rename(&temporary, &path).map_err(|error| error.to_string())?;
    prune(cache);
    Ok(path)
}

/// The middle `width` × `height` of the first frame (all of it where it is smaller), as straight
/// RGBA.
fn crop(image: &oxyos_image::Image, width: u32, height: u32) -> (Vec<u8>, u32, u32) {
    let (w, h) = (width.min(image.width), height.min(image.height));
    let (left, top) = ((image.width - w) / 2, (image.height - h) / 2);
    let mut out = Vec::with_capacity(w as usize * h as usize * 4);
    for y in top..top + h {
        let start = (y as usize * image.width as usize + left as usize) * 4;
        for pixel in image.pixels[start..start + w as usize * 4].chunks_exact(4) {
            let a = pixel[3];
            // Premultiplied to straight.
            let straight = |c: u8| {
                if a == 0 {
                    0
                } else {
                    ((u32::from(c) * 255 + u32::from(a) / 2) / u32::from(a)).min(255) as u8
                }
            };
            out.extend_from_slice(&[
                straight(pixel[0]),
                straight(pixel[1]),
                straight(pixel[2]),
                a,
            ]);
        }
    }
    (out, w, h)
}

fn write_png(path: &Path, rgba: &[u8], width: u32, height: u32) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|error| error.to_string())?;
    let mut writer = std::io::BufWriter::new(file);
    {
        let mut encoder = png::Encoder::new(&mut writer, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut png = encoder.write_header().map_err(|error| error.to_string())?;
        png.write_image_data(rgba)
            .map_err(|error| error.to_string())?;
    }
    writer.flush().map_err(|error| error.to_string())
}

/// Removes the oldest thumbnails past [`MAX_KEPT`].
fn prune(cache: &Path) {
    let Ok(entries) = std::fs::read_dir(cache) else {
        return;
    };
    let mut kept: Vec<(std::time::SystemTime, PathBuf)> = entries
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".png"))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    if kept.len() <= MAX_KEPT {
        return;
    }
    kept.sort();
    for (_, path) in &kept[..kept.len() - MAX_KEPT] {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
pub(crate) fn write_png_for_tests(path: &Path, rgba: &[u8], width: u32, height: u32) {
    write_png(path, rgba, width, height).unwrap();
}
