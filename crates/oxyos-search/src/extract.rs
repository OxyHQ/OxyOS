//! The text of a file, for its contents to be searched (docs/adr/0051-search.mdx, decision 3), by
//! proven readers only:
//!
//! - **Text, Markdown and source code:** the file's own bytes, UTF-8 (or UTF-16 with its byte
//!   order mark), what is not UTF-8 replaced; a file with a NUL in its first 8 KiB is binary and
//!   has none.
//! - **Office documents and EPUB** are ZIP archives of XML: the `zip` crate opens them and
//!   `quick-xml` reads the parts that hold their text (Word's `word/document.xml`, PowerPoint's
//!   slides in order, Excel's shared strings, OpenDocument's `content.xml`, an EPUB's XHTML).
//! - **PDF:** MuPDF's `mutool draw -F txt`, a program of its own (the image's `mupdf-tools`,
//!   toolkit-free), run with the indexer's idle scheduling, under `prlimit` (1 GiB of memory, 20 s
//!   of CPU) and killed after 30 s: a PDF crafted to hang or exhaust its reader never reaches the
//!   service. Without `mutool`, a PDF is found by its name alone.
//!
//! At most [`MAX_TEXT`] bytes of text are kept for an item; what is beyond is not searched.

use crate::category::Category;
use quick_xml::events::Event;
use std::io::{BufReader, Read};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The most text kept for an item, in bytes.
pub const MAX_TEXT: usize = 1 << 20;
/// Files larger than this are not read for their text (a log of gigabytes, a disk image renamed).
pub const MAX_FILE: u64 = 256 << 20;
/// How long a PDF's reader may run.
const PDF_TIMEOUT: Duration = Duration::from_secs(30);

/// The text of the file at `path` of `category` and lowercase `extension`, if it has any that is
/// read.
pub fn text(path: &Path, category: Category, extension: Option<&str>, size: u64) -> Option<String> {
    if size == 0 || size > MAX_FILE {
        return None;
    }
    match (category, extension) {
        (Category::Text | Category::Source, _) => plain(path),
        (Category::Pdf, _) => pdf(path),
        (_, Some("docx")) => office(path, Office::Word),
        (_, Some("pptx")) => office(path, Office::PowerPoint),
        (_, Some("xlsx")) => office(path, Office::Excel),
        (_, Some("odt" | "ods" | "odp")) => office(path, Office::OpenDocument),
        (_, Some("epub")) => office(path, Office::Epub),
        _ => None,
    }
}

/// A text file's text.
fn plain(path: &Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take(MAX_TEXT as u64).read_to_end(&mut bytes).ok()?;
    if let Some(units) = bytes
        .strip_prefix(&[0xFF, 0xFE])
        .map(|rest| (rest, true))
        .or_else(|| bytes.strip_prefix(&[0xFE, 0xFF]).map(|rest| (rest, false)))
    {
        let (rest, little) = units;
        let words: Vec<u16> = rest
            .chunks_exact(2)
            .map(|pair| {
                if little {
                    u16::from_le_bytes([pair[0], pair[1]])
                } else {
                    u16::from_be_bytes([pair[0], pair[1]])
                }
            })
            .collect();
        return Some(String::from_utf16_lossy(&words));
    }
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&bytes);
    if bytes[..bytes.len().min(8192)].contains(&0) {
        return None;
    }
    // A read cut in the middle of a character loses it alone.
    Some(String::from_utf8_lossy(bytes).into_owned())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Office {
    Word,
    PowerPoint,
    Excel,
    OpenDocument,
    Epub,
}

/// The text of an office document or an EPUB.
fn office(path: &Path, kind: Office) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let mut archive = zip::ZipArchive::new(BufReader::new(file)).ok()?;
    let mut names: Vec<String> = archive
        .file_names()
        .filter_map(|name| name.ok().map(|name| name.into_owned()))
        .filter(|name| match kind {
            Office::Word => name == "word/document.xml",
            Office::PowerPoint => slide_number(name).is_some(),
            Office::Excel => name == "xl/sharedStrings.xml",
            Office::OpenDocument => name == "content.xml",
            Office::Epub => {
                let lower = name.to_ascii_lowercase();
                lower.ends_with(".xhtml") || lower.ends_with(".html") || lower.ends_with(".htm")
            }
        })
        .collect();
    // Slides in their order (slide2 before slide10); an EPUB's chapters by name.
    names.sort_by_key(|name| (slide_number(name), name.clone()));
    let mut text = String::new();
    for name in names {
        if text.len() >= MAX_TEXT {
            break;
        }
        let Ok(part) = archive.by_name(&name) else {
            continue;
        };
        // A part claiming more than it may hold is read only so far.
        xml_text(BufReader::new(part.take(64 << 20)), &mut text);
        text.push('\n');
    }
    truncate(&mut text);
    (!text.trim().is_empty()).then_some(text)
}

/// `ppt/slides/slideN.xml`'s N.
fn slide_number(name: &str) -> Option<u32> {
    name.strip_prefix("ppt/slides/slide")?
        .strip_suffix(".xml")?
        .parse()
        .ok()
}

/// Elements whose end is a break between words (paragraphs, cells, rows, slides' shapes).
const BREAKS: &[&str] = &[
    "p", "h", "br", "ta", "tc", "tr", "row", "c", "si", "li", "div", "td", "th",
    "h1", "h2", "h3", "h4", "h5", "h6", "title", "table-cell", "line-break", "s",
];

/// Appends the text of an XML document to `text`: every text node outside `script` and `style`,
/// a break after each paragraph-like element, entities resolved. A malformed document gives what
/// was read before its fault.
fn xml_text<R: std::io::BufRead>(reader: R, text: &mut String) {
    let mut reader = quick_xml::Reader::from_reader(reader);
    let mut buffer = Vec::new();
    let mut hidden = 0usize;
    loop {
        if text.len() >= MAX_TEXT {
            return;
        }
        let event = match reader.read_event_into(&mut buffer) {
            Ok(Event::Eof) | Err(_) => return,
            Ok(event) => event,
        };
        match event {
            Event::Start(start) => {
                if hiding(start.local_name().into_inner()) {
                    hidden += 1;
                }
            }
            Event::End(end) => {
                let name = end.local_name();
                let name = name.into_inner();
                if hiding(name) {
                    hidden = hidden.saturating_sub(1);
                } else if BREAKS.contains(&name) {
                    text.push('\n');
                }
            }
            Event::Empty(empty) => {
                let name = empty.local_name();
                let name = name.into_inner();
                if BREAKS.contains(&name) {
                    text.push(' ');
                }
            }
            Event::Text(content) if hidden == 0 => {
                text.push_str(&content.xml_content(quick_xml::XmlVersion::Implicit1_0));
            }
            Event::CData(content) if hidden == 0 => {
                text.push_str(&content.xml_content(quick_xml::XmlVersion::Implicit1_0));
            }
            Event::GeneralRef(reference) if hidden == 0 => {
                if let Ok(Some(c)) = reference.resolve_char_ref() {
                    text.push(c);
                } else {
                    // The five XML predefines; any other entity is the DTD's and says nothing here.
                    text.push_str(match &*reference {
                        "amp" => "&",
                        "lt" => "<",
                        "gt" => ">",
                        "quot" => "\"",
                        "apos" => "'",
                        _ => " ",
                    });
                }
            }
            _ => {}
        }
        buffer.clear();
    }
}

/// Whether an element's text is no text a person reads (a script, a style sheet).
fn hiding(name: &str) -> bool {
    name == "script" || name == "style"
}

/// `text` cut to at most [`MAX_TEXT`] bytes, on a character's boundary.
fn truncate(text: &mut String) {
    if text.len() > MAX_TEXT {
        let mut end = MAX_TEXT;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
}

/// The program that reads PDFs: `mutool` on the path, or `OXYOS_SEARCH_MUTOOL`.
fn mutool() -> Option<std::path::PathBuf> {
    if let Some(program) = std::env::var_os("OXYOS_SEARCH_MUTOOL") {
        return Some(program.into());
    }
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|folder| folder.join("mutool"))
            .find(|program| program.is_file())
    })
}

/// Whether PDFs are read for their text here.
pub fn reads_pdf() -> bool {
    mutool().is_some()
}

/// A PDF's text, from MuPDF.
fn pdf(path: &Path) -> Option<String> {
    let program = mutool()?;
    let limits = Path::new("/usr/bin/prlimit");
    let mut command = if limits.is_file() {
        let mut command = Command::new(limits);
        command
            .arg(format!("--as={}", 1u64 << 30))
            .arg("--cpu=20")
            .arg("--")
            .arg(program);
        command
    } else {
        Command::new(program)
    };
    let mut child = command
        .args(["draw", "-q", "-F", "txt", "-o", "-"])
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut output = child.stdout.take()?;
    let deadline = Instant::now() + PDF_TIMEOUT;
    let mut bytes = Vec::new();
    let mut chunk = vec![0u8; 64 * 1024];
    let read = loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break false;
        }
        let timeout = rustix::event::Timespec {
            tv_sec: left.as_secs() as i64,
            tv_nsec: i64::from(left.subsec_nanos()),
        };
        let mut fds = [rustix::event::PollFd::new(
            &output,
            rustix::event::PollFlags::IN,
        )];
        match rustix::event::poll(&mut fds, Some(&timeout)) {
            Ok(0) => break false,
            Ok(_) => {}
            Err(rustix::io::Errno::INTR) => continue,
            Err(_) => break false,
        }
        match output.read(&mut chunk) {
            Ok(0) => break true,
            Ok(count) => {
                if bytes.len() < MAX_TEXT {
                    bytes.extend_from_slice(&chunk[..count]);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => break false,
        }
    };
    if !read {
        let _ = child.kill();
    }
    drop(output);
    let status = child.wait().ok()?;
    if !read || !status.success() {
        return None;
    }
    bytes.truncate(MAX_TEXT);
    let mut text = String::from_utf8_lossy(&bytes).replace('\u{c}', "\n");
    truncate(&mut text);
    (!text.trim().is_empty()).then_some(text)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Write;

    /// A ZIP archive at `path` holding `parts` (name, contents).
    pub(crate) fn zip_file(path: &Path, parts: &[(&str, &str)]) {
        let file = std::fs::File::create(path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        for (name, contents) in parts {
            writer
                .start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(contents.as_bytes()).unwrap();
        }
        writer.finish().unwrap();
    }

    /// A one-page PDF saying `words`, made by hand to the PDF 1.4 syntax (its cross-reference
    /// table's offsets computed).
    pub(crate) fn pdf_file(path: &Path, words: &str) {
        let stream = format!("BT /F1 12 Tf 72 720 Td ({words}) Tj ET");
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_string(),
            format!("<< /Length {} >>\nstream\n{stream}\nendstream", stream.len()),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
        ];
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (n, object) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", n + 1).as_bytes());
        }
        let xref = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
        for offset in offsets {
            out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );
        std::fs::write(path, out).unwrap();
    }

    fn of(path: &Path) -> Option<String> {
        let name = path.file_name().unwrap().to_str().unwrap();
        let extension = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
        let category = Category::of_file(oxyos_files::mime::of_name(name), extension.as_deref());
        let size = std::fs::metadata(path).unwrap().len();
        text(path, category, extension.as_deref(), size)
    }

    #[test]
    fn text_and_code_are_their_bytes_and_binaries_have_none() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("notes.md"), "# Viaje a Córdoba\nbillete").unwrap();
        std::fs::write(dir.path().join("main.rs"), "fn quarterly() {}").unwrap();
        std::fs::write(dir.path().join("blob.txt"), b"abc\0def").unwrap();
        let mut utf16 = vec![0xFF, 0xFE];
        utf16.extend("hola mundo".encode_utf16().flat_map(u16::to_le_bytes));
        std::fs::write(dir.path().join("wide.txt"), utf16).unwrap();
        assert_eq!(of(&dir.path().join("notes.md")).unwrap(), "# Viaje a Córdoba\nbillete");
        assert_eq!(of(&dir.path().join("main.rs")).unwrap(), "fn quarterly() {}");
        assert_eq!(of(&dir.path().join("blob.txt")), None);
        assert_eq!(of(&dir.path().join("wide.txt")).unwrap(), "hola mundo");
    }

    #[test]
    fn office_documents_give_their_words() {
        let dir = tempfile::tempdir().unwrap();
        let docx = dir.path().join("Letter.docx");
        zip_file(
            &docx,
            &[(
                "word/document.xml",
                r#"<?xml version="1.0"?><w:document xmlns:w="w"><w:body><w:p><w:r><w:t>Dear</w:t></w:r><w:r><w:t xml:space="preserve"> Ana &amp; Luis</w:t></w:r></w:p><w:p><w:r><w:t>Invoice&#160;42</w:t></w:r></w:p></w:body></w:document>"#,
            )],
        );
        let text = of(&docx).unwrap();
        assert!(text.contains("Dear Ana & Luis\n"), "{text:?}");
        assert!(text.contains("Invoice\u{a0}42"), "{text:?}");

        let pptx = dir.path().join("Talk.pptx");
        zip_file(
            &pptx,
            &[
                ("ppt/slides/slide10.xml", r#"<p:sld xmlns:a="a" xmlns:p="p"><a:t>Last</a:t></p:sld>"#),
                ("ppt/slides/slide2.xml", r#"<p:sld xmlns:a="a" xmlns:p="p"><a:t>Second</a:t></p:sld>"#),
                ("ppt/slides/slide1.xml", r#"<p:sld xmlns:a="a" xmlns:p="p"><a:t>First</a:t></p:sld>"#),
            ],
        );
        let text = of(&pptx).unwrap();
        let order: Vec<&str> = text.split_whitespace().collect();
        assert_eq!(order, ["First", "Second", "Last"]);

        let xlsx = dir.path().join("Budget.xlsx");
        zip_file(
            &xlsx,
            &[("xl/sharedStrings.xml", r#"<sst><si><t>Groceries</t></si><si><t>Rent</t></si></sst>"#)],
        );
        assert_eq!(of(&xlsx).unwrap().split_whitespace().collect::<Vec<_>>(), ["Groceries", "Rent"]);

        let odt = dir.path().join("Novel.odt");
        zip_file(
            &odt,
            &[(
                "content.xml",
                r#"<office:document-content xmlns:office="o" xmlns:text="t"><office:body><text:h>Capítulo</text:h><text:p>Érase<text:s/>una vez</text:p></office:body></office:document-content>"#,
            )],
        );
        let text = of(&odt).unwrap();
        assert!(text.contains("Capítulo\n"), "{text:?}");
        assert!(text.contains("Érase una vez"), "{text:?}");

        let epub = dir.path().join("Book.epub");
        zip_file(
            &epub,
            &[(
                "OEBPS/ch1.xhtml",
                r#"<html><head><style>p{}</style><script>var x;</script></head><body><p>Call me Ishmael.</p></body></html>"#,
            )],
        );
        let text = of(&epub).unwrap();
        assert!(text.contains("Call me Ishmael."), "{text:?}");
        assert!(!text.contains("var x") && !text.contains("p{}"), "{text:?}");
    }

    #[test]
    fn a_broken_archive_has_no_text() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("broken.docx"), "not a zip").unwrap();
        assert_eq!(of(&dir.path().join("broken.docx")), None);
    }

    #[test]
    fn a_pdf_is_read_by_mupdf() {
        if !reads_pdf() {
            eprintln!("skipped: no mutool on this machine (mupdf-tools)");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let pdf = dir.path().join("Invoice.pdf");
        pdf_file(&pdf, "Quarterly invoice for Atlas");
        let text = of(&pdf).unwrap();
        assert!(text.contains("Quarterly invoice for Atlas"), "{text:?}");
        std::fs::write(dir.path().join("broken.pdf"), "%PDF-1.4 nonsense").unwrap();
        assert_eq!(of(&dir.path().join("broken.pdf")), None);
    }
}
