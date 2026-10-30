//! A file's type from its name, as `expo-file-system`'s `File.type` gives it (Android's
//! `MimeTypeMap`, a table of extensions) and as a download into a folder names a file from its
//! response's type. OxyOS's image has no shared-mime-info, so the table is the platform's own:
//! the IANA media types of the extensions applications meet, by lowercase extension.

/// Extensions and their media types, sorted by extension.
const TYPES: &[(&str, &str)] = &[
    ("7z", "application/x-7z-compressed"),
    ("aac", "audio/aac"),
    ("avi", "video/x-msvideo"),
    ("avif", "image/avif"),
    ("bin", "application/octet-stream"),
    ("bmp", "image/bmp"),
    ("bz2", "application/x-bzip2"),
    ("c", "text/x-c"),
    ("css", "text/css"),
    ("csv", "text/csv"),
    ("doc", "application/msword"),
    (
        "docx",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    ),
    ("epub", "application/epub+zip"),
    ("flac", "audio/flac"),
    ("gif", "image/gif"),
    ("gz", "application/gzip"),
    ("heic", "image/heic"),
    ("htm", "text/html"),
    ("html", "text/html"),
    ("ico", "image/vnd.microsoft.icon"),
    ("ics", "text/calendar"),
    ("jpeg", "image/jpeg"),
    ("jpg", "image/jpeg"),
    ("js", "text/javascript"),
    ("json", "application/json"),
    ("m4a", "audio/mp4"),
    ("md", "text/markdown"),
    ("mid", "audio/midi"),
    ("mjs", "text/javascript"),
    ("mkv", "video/x-matroska"),
    ("mov", "video/quicktime"),
    ("mp3", "audio/mpeg"),
    ("mp4", "video/mp4"),
    ("mpeg", "video/mpeg"),
    ("odp", "application/vnd.oasis.opendocument.presentation"),
    ("ods", "application/vnd.oasis.opendocument.spreadsheet"),
    ("odt", "application/vnd.oasis.opendocument.text"),
    ("oga", "audio/ogg"),
    ("ogg", "audio/ogg"),
    ("ogv", "video/ogg"),
    ("opus", "audio/opus"),
    ("otf", "font/otf"),
    ("pdf", "application/pdf"),
    ("png", "image/png"),
    ("ppt", "application/vnd.ms-powerpoint"),
    (
        "pptx",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation",
    ),
    ("rar", "application/vnd.rar"),
    ("rtf", "application/rtf"),
    ("sh", "application/x-sh"),
    ("svg", "image/svg+xml"),
    ("tar", "application/x-tar"),
    ("tif", "image/tiff"),
    ("tiff", "image/tiff"),
    ("ts", "video/mp2t"),
    ("ttf", "font/ttf"),
    ("txt", "text/plain"),
    ("vcf", "text/vcard"),
    ("wav", "audio/wav"),
    ("weba", "audio/webm"),
    ("webm", "video/webm"),
    ("webp", "image/webp"),
    ("woff", "font/woff"),
    ("woff2", "font/woff2"),
    ("xls", "application/vnd.ms-excel"),
    (
        "xlsx",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    ),
    ("xml", "application/xml"),
    ("xz", "application/x-xz"),
    ("yaml", "application/yaml"),
    ("yml", "application/yaml"),
    ("zip", "application/zip"),
];

/// The media type of a file named `name`, from its extension; none if it has none the table
/// knows.
pub fn of_name(name: &str) -> Option<&'static str> {
    let (stem, extension) = name.rsplit_once('.')?;
    if stem.is_empty() {
        return None;
    }
    let extension = extension.to_ascii_lowercase();
    TYPES
        .binary_search_by(|(known, _)| (*known).cmp(extension.as_str()))
        .ok()
        .map(|at| TYPES[at].1)
}

/// The extension a type with several takes, as files of it are usually named.
const PREFERRED: &[(&str, &str)] = &[
    ("application/yaml", "yaml"),
    ("audio/ogg", "ogg"),
    ("image/jpeg", "jpg"),
    ("image/tiff", "tiff"),
    ("text/html", "html"),
    ("text/javascript", "js"),
];

/// The extension a file of media type `media` takes (its parameters aside), if the table knows
/// one.
pub fn extension_of(media: &str) -> Option<&'static str> {
    let media = media.split(';').next()?.trim().to_ascii_lowercase();
    if let Some((_, extension)) = PREFERRED.iter().find(|(known, _)| *known == media) {
        return Some(extension);
    }
    TYPES
        .iter()
        .find(|(_, known)| *known == media)
        .map(|(extension, _)| *extension)
}

#[cfg(test)]
pub(crate) fn sorted() -> bool {
    TYPES.windows(2).all(|pair| pair[0].0 < pair[1].0)
}
