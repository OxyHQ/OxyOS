//! Reports what each DRM card offers, as JSON: driver, outputs (connected or not, modes) and
//! planes with the modifiers each accepts for the compositor's formats (XRGB8888, ARGB8888).
//! Read-only: it needs no DRM master and leaves the running session alone.
//!
//!     cargo run -p oxy-kms-probe [-- /dev/dri/cardN ...]

use std::path::PathBuf;

fn main() {
    let mut paths: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    if paths.is_empty() {
        let mut cards: Vec<PathBuf> = std::fs::read_dir("/dev/dri")
            .map(|dir| dir.filter_map(Result::ok).map(|e| e.path()).collect())
            .unwrap_or_default();
        cards.retain(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("card"))
        });
        cards.sort();
        paths = cards;
    }
    let mut reports = Vec::new();
    for path in &paths {
        reports.push(match report(path) {
            Ok(report) => report,
            Err(error) => format!(
                "{{\"card\":{:?},\"error\":{:?}}}",
                path.display().to_string(),
                error.to_string()
            ),
        });
    }
    println!("{{\"cards\":[{}]}}", reports.join(","));
}

fn report(path: &std::path::Path) -> std::io::Result<String> {
    use oxy_kms::{Card, DrmFourcc, PlaneKind};
    use std::fmt::Write;
    let card = Card::open(path)?;
    let mut out = format!(
        "{{\"card\":{:?},\"driver\":{:?},\"outputs\":[",
        path.display().to_string(),
        card.driver_name()?
    );
    for (i, output) in card.outputs()?.iter().enumerate() {
        let preferred = output
            .modes
            .iter()
            .find(|m| m.preferred)
            .or(output.modes.first());
        let _ = write!(
            out,
            "{}{{\"name\":{:?},\"connected\":{},\"modes\":{},\"preferred\":{}}}",
            if i > 0 { "," } else { "" },
            output.name,
            output.connected,
            output.modes.len(),
            preferred.map_or("null".into(), |m| format!(
                "[{},{},{}]",
                m.width, m.height, m.refresh_hz
            ))
        );
    }
    out.push_str("],\"planes\":[");
    for (i, plane) in card.planes()?.iter().enumerate() {
        let kind = match plane.kind {
            PlaneKind::Primary => "primary",
            PlaneKind::Overlay => "overlay",
            PlaneKind::Cursor => "cursor",
            PlaneKind::Unknown => "unknown",
        };
        let modifiers = |format: DrmFourcc| -> String {
            let list: Vec<String> = plane
                .formats
                .iter()
                .filter(|(f, _)| *f == format)
                .map(|(_, m)| format!("\"{m:#x}\""))
                .collect();
            format!("[{}]", list.join(","))
        };
        let _ = write!(
            out,
            "{}{{\"id\":{},\"type\":\"{kind}\",\"pairs\":{},\"xrgb8888\":{},\"argb8888\":{}}}",
            if i > 0 { "," } else { "" },
            plane.id,
            plane.formats.len(),
            modifiers(DrmFourcc::Xrgb8888),
            modifiers(DrmFourcc::Argb8888)
        );
    }
    out.push_str("]}");
    Ok(out)
}
