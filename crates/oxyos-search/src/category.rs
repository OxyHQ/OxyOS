//! What kind of thing an item is, as Spotlight groups its results (Applications, Documents,
//! Folders, PDF Documents, System Settings, …): each item has one category, its section in the
//! search panel, a filter of the query language (`kind:pdf`) and a switch of Settings › Spotlight.

use serde::{Deserialize, Serialize};

/// An item's category.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Category {
    Application,
    Settings,
    Folder,
    Document,
    Pdf,
    Text,
    Source,
    Presentation,
    Spreadsheet,
    Image,
    Movie,
    Music,
    Archive,
    Contact,
    Event,
    Mail,
    Message,
    Other,
}

impl Category {
    /// Every category, in the order Settings › Spotlight lists them.
    pub const ALL: [Category; 18] = [
        Category::Application,
        Category::Settings,
        Category::Folder,
        Category::Document,
        Category::Pdf,
        Category::Text,
        Category::Source,
        Category::Presentation,
        Category::Spreadsheet,
        Category::Image,
        Category::Movie,
        Category::Music,
        Category::Archive,
        Category::Contact,
        Category::Event,
        Category::Mail,
        Category::Message,
        Category::Other,
    ];

    /// Its name in the protocol and the index.
    pub fn name(self) -> &'static str {
        match self {
            Category::Application => "application",
            Category::Settings => "settings",
            Category::Folder => "folder",
            Category::Document => "document",
            Category::Pdf => "pdf",
            Category::Text => "text",
            Category::Source => "source",
            Category::Presentation => "presentation",
            Category::Spreadsheet => "spreadsheet",
            Category::Image => "image",
            Category::Movie => "movie",
            Category::Music => "music",
            Category::Archive => "archive",
            Category::Contact => "contact",
            Category::Event => "event",
            Category::Mail => "mail",
            Category::Message => "message",
            Category::Other => "other",
        }
    }

    /// The category named `name` in the protocol.
    pub fn parse(name: &str) -> Option<Category> {
        Category::ALL
            .into_iter()
            .find(|category| category.name() == name)
    }

    /// The category a person names after `kind:` (or `tipo:`), in English or Spanish, singular or
    /// plural, as a Mac's Spotlight takes `kind:pdf`, `kind:image`, `kind:folder`.
    pub fn from_word(word: &str) -> Option<Category> {
        let category = match word {
            "app" | "apps" | "application" | "applications" | "aplicacion" | "aplicaciones"
            | "aplicación" | "programa" | "programas" => Category::Application,
            "settings" | "setting" | "preferences" | "ajustes" | "ajuste" | "preferencias"
            | "configuracion" | "configuración" => Category::Settings,
            "folder" | "folders" | "directory" | "carpeta" | "carpetas" => Category::Folder,
            "document" | "documents" | "doc" | "docs" | "documento" | "documentos" => {
                Category::Document
            }
            "pdf" | "pdfs" => Category::Pdf,
            "text" | "texts" | "txt" | "texto" | "textos" | "markdown" | "md" => Category::Text,
            "source" | "code" | "developer" | "codigo" | "código" | "fuente" => Category::Source,
            "presentation" | "presentations" | "slides" | "presentacion" | "presentación"
            | "presentaciones" | "diapositivas" => Category::Presentation,
            "spreadsheet" | "spreadsheets" | "sheet" | "sheets" | "hoja" | "hojas"
            | "hojadecalculo" => Category::Spreadsheet,
            "image" | "images" | "picture" | "pictures" | "photo" | "photos" | "imagen"
            | "imagenes" | "imágenes" | "foto" | "fotos" => Category::Image,
            "movie" | "movies" | "video" | "videos" | "vídeo" | "vídeos" | "pelicula"
            | "película" | "peliculas" | "películas" => Category::Movie,
            "music" | "audio" | "song" | "songs" | "musica" | "música" | "cancion" | "canción"
            | "canciones" => Category::Music,
            "archive" | "archives" | "zip" | "comprimido" | "comprimidos" => Category::Archive,
            "contact" | "contacts" | "contacto" | "contactos" => Category::Contact,
            "event" | "events" | "evento" | "eventos" | "reminder" | "reminders"
            | "recordatorio" | "recordatorios" => Category::Event,
            "mail" | "email" | "emails" | "correo" | "correos" => Category::Mail,
            "message" | "messages" | "mensaje" | "mensajes" | "chat" => Category::Message,
            "other" | "otro" | "otros" => Category::Other,
            _ => return None,
        };
        Some(category)
    }

    /// The category of a file of media type `media` (from its name, `oxyos_files::mime`), or of
    /// its extension where the media type says too little (source code is text to it).
    pub fn of_file(media: Option<&str>, extension: Option<&str>) -> Category {
        if let Some(extension) = extension {
            if SOURCE_EXTENSIONS.contains(&extension) {
                return Category::Source;
            }
            if TEXT_EXTENSIONS.contains(&extension) {
                return Category::Text;
            }
            if let Some(category) = DOCUMENT_EXTENSIONS
                .iter()
                .find(|(known, _)| *known == extension)
                .map(|(_, category)| *category)
            {
                return category;
            }
        }
        let Some(media) = media else {
            return Category::Other;
        };
        match media {
            "application/pdf" => Category::Pdf,
            "text/markdown" | "text/plain" | "text/csv" => Category::Text,
            "application/zip"
            | "application/gzip"
            | "application/x-7z-compressed"
            | "application/x-bzip2"
            | "application/x-tar"
            | "application/x-xz"
            | "application/vnd.rar" => Category::Archive,
            "application/msword" | "application/rtf" | "application/epub+zip" => Category::Document,
            _ if media.starts_with("image/") => Category::Image,
            _ if media.starts_with("video/") => Category::Movie,
            _ if media.starts_with("audio/") => Category::Music,
            _ if media.starts_with("text/") => Category::Source,
            _ => Category::Other,
        }
    }
}

/// Source code, a Mac's "Developer" kind: indexed for its text.
const SOURCE_EXTENSIONS: &[&str] = &[
    "c", "cc", "cpp", "cxx", "h", "hh", "hpp", "rs", "go", "py", "rb", "java", "kt", "kts",
    "swift", "m", "mm", "js", "mjs", "cjs", "jsx", "ts", "tsx", "css", "scss", "html", "htm",
    "json", "toml", "yaml", "yml", "xml", "sh", "bash", "zsh", "fish", "sql", "lua", "php", "pl",
    "cs", "fs", "hs", "ml", "ex", "exs", "erl", "clj", "scala", "dart", "zig", "nim", "vue",
    "svelte", "mdx", "cmake", "mk", "ini", "cfg", "conf", "gradle", "proto", "graphql", "tex",
    "diff", "patch",
];

/// Plain text, a Mac's "Text" kind.
const TEXT_EXTENSIONS: &[&str] = &[
    "txt", "text", "md", "markdown", "rst", "org", "log", "csv", "tsv",
];

/// Office documents by extension: their media types are long and some are missing from the
/// table.
const DOCUMENT_EXTENSIONS: &[(&str, Category)] = &[
    ("doc", Category::Document),
    ("docx", Category::Document),
    ("odt", Category::Document),
    ("rtf", Category::Document),
    ("epub", Category::Document),
    ("pages", Category::Document),
    ("ppt", Category::Presentation),
    ("pptx", Category::Presentation),
    ("odp", Category::Presentation),
    ("key", Category::Presentation),
    ("xls", Category::Spreadsheet),
    ("xlsx", Category::Spreadsheet),
    ("ods", Category::Spreadsheet),
    ("numbers", Category::Spreadsheet),
    ("tar", Category::Archive),
    ("xz", Category::Archive),
    ("zst", Category::Archive),
    ("rar", Category::Archive),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_name_parses_back() {
        for category in Category::ALL {
            assert_eq!(Category::parse(category.name()), Some(category));
            assert_eq!(Category::from_word(category.name()), Some(category));
        }
    }

    #[test]
    fn files_take_their_kinds() {
        let of = |name: &str| {
            let extension = name.rsplit_once('.').map(|(_, e)| e);
            Category::of_file(oxyos_files::mime::of_name(name), extension)
        };
        assert_eq!(of("Report.pdf"), Category::Pdf);
        assert_eq!(of("notes.md"), Category::Text);
        assert_eq!(of("main.rs"), Category::Source);
        assert_eq!(of("Budget.xlsx"), Category::Spreadsheet);
        assert_eq!(of("Talk.odp"), Category::Presentation);
        assert_eq!(of("Letter.docx"), Category::Document);
        assert_eq!(of("beach.jpg"), Category::Image);
        assert_eq!(of("song.flac"), Category::Music);
        assert_eq!(of("film.mkv"), Category::Movie);
        assert_eq!(of("backup.zip"), Category::Archive);
        assert_eq!(of("unknown.qqq"), Category::Other);
    }
}
