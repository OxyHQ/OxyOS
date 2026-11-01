//! Reports what each DRM card offers, as JSON: driver, outputs (connected or not, modes) and
//! planes with the modifiers each accepts for the compositor's formats (XRGB8888, ARGB8888).
//! Read-only: it needs no DRM master and leaves the running session alone.
//!
//!     cargo run -p oxyos-kms-probe [-- /dev/dri/cardN ...]
//!
//! Without cards named, the cards that drive displays (sysfs: those with connectors); a GPU that
//! drives none is not opened, which would wake it (docs/adr/0031-gpu-buffers.mdx, decision 14).

use std::path::{Path, PathBuf};

fn main() {
    let mut paths: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    if paths.is_empty() {
        paths = oxyos_gpu_device::display_cards(Path::new(oxyos_gpu_device::SYSTEM_ROOT));
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
    use oxyos_kms::{Card, DrmFourcc, PlaneKind};
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
