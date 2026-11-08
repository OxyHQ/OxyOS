//! The index (docs/adr/0051-search.mdx, decisions 2 and 5): tantivy's segments in
//! `$XDG_CACHE_HOME/oxyos/search/index-N`, one document per item, and the questions asked of it.
//!
//! - **What an item is to it:** its [`Record`] (its id, where it comes from, its category, its
//!   titles in each language, keywords, tags, dates and size), stored whole as JSON and indexed in
//!   fields: its titles as words and as every prefix of each word (the edge n-grams that make
//!   `rep` find `Report` as it is typed, as search-as-you-type engines index them), its whole
//!   title folded (an exact match), its keywords and tags likewise, its tags as filters, its text
//!   as words with positions (phrases), every folder above a file (`within`, and a folder's items
//!   removed in one step), its dates and size. Words are folded: lowercase, accents and other marks
//!   removed (`camion` finds `Camión`).
//! - **The text is not stored:** a file's text is read again from the file for a result's snippet,
//!   so the index holds the words alone and stays small. A donated item's text, which nothing can
//!   read again, is kept in its stored record, so a rebuild carries it over.
//! - **Its directories** ([`Layout`]): the one answering, and a rebuild's beside it while it is
//!   built; which is which is one small file written whole, so the index on the disk is always one
//!   or the other, whenever the machine stops.
//! - **Ranking:** BM25 over the fields, the title weighing most, then keywords and tags, then the
//!   text; then each candidate weighed by its category (applications first, as Spotlight puts
//!   them), by how recently it changed or was used, and by how often the person opened it. The top
//!   hit is the best item whose title, keywords or tags have every word typed as a word's beginning.

use crate::category::Category;
use crate::query::{Parsed, Span};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::ops::Bound;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use tantivy::collector::{DocSetCollector, TopDocs};
use tantivy::query::{
    AllQuery, BooleanQuery, BoostQuery, Occur, PhrasePrefixQuery, PhraseQuery, Query, RangeQuery,
    TermQuery,
};
use tantivy::schema::{
    Field, IndexRecordOption, Schema, TextFieldIndexing, TextOptions, Value, FAST, INDEXED, STORED,
    STRING,
};
use tantivy::tokenizer::{
    AsciiFoldingFilter, LowerCaser, RawTokenizer, RemoveLongFilter, SimpleTokenizer, TextAnalyzer,
    Token, TokenFilter, TokenStream, Tokenizer,
};
use tantivy::{IndexReader, IndexWriter, ReloadPolicy, TantivyDocument, Term};

/// The index's layout: a change makes a new index beside the old, which is removed.
pub const VERSION: u32 = 1;
/// What the writer may hold before it writes a segment (tantivy's least).
const WRITER_BUDGET: usize = 15_000_000;
/// The longest word kept; longer runs (hashes, base64) are no words.
const LONGEST_WORD: usize = 48;
/// The longest prefix indexed of a word.
const LONGEST_PREFIX: usize = 20;
/// Candidates weighed beyond BM25 for each question.
const CANDIDATES: usize = 400;

/// Where an item comes from, which decides who sees it (decision 6).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Source {
    /// A file or folder in the person's places.
    File,
    /// An application the session lists.
    Application,
    /// An item an application's package declares (`searchable.json`: Settings' pages).
    Declared,
    /// An item an application donated while running (Core Spotlight's).
    Donated,
}

impl Source {
    fn name(&self) -> &'static str {
        match self {
            Source::File => "file",
            Source::Application => "application",
            Source::Declared => "declared",
            Source::Donated => "donated",
        }
    }
}

/// An item, as the index keeps it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    pub id: String,
    pub source: Source,
    /// The application whose item it is (declared or donated).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    pub category: Category,
    /// Its title in each language, by BCP 47 tag; `""` is the one for any language.
    pub titles: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keywords: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// A file's or folder's path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub application: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    /// Milliseconds since the epoch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<i64>,
    /// A file's modification time in nanoseconds when it was read: whether it changed since.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changed: Option<i64>,
}

impl Record {
    /// Its title in `language` (`es-ES` takes `es`), else the one for any language, else
    /// English, else any.
    pub fn title(&self, language: &str) -> &str {
        let base = language.split(['-', '_']).next().unwrap_or("");
        [language, base, "", "en"]
            .iter()
            .find_map(|key| self.titles.get(*key))
            .or_else(|| self.titles.values().next())
            .map_or("", String::as_str)
    }
}

/// Who asks, which decides what they see (decision 6).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Viewer {
    /// The shell: everything, but the categories the person turned off.
    pub shell: bool,
    /// An application the session launched: its own items besides the files.
    pub application: Option<String>,
    /// Categories never shown to it (the shell's switches in Settings › Spotlight).
    pub hidden: Vec<Category>,
}

/// A question.
#[derive(Clone, Debug, Default)]
pub struct Question {
    pub parsed: Parsed,
    /// Only inside these folders.
    pub within: Vec<PathBuf>,
    /// Only these categories (the asker's own choice, besides the query's `kind:`).
    pub categories: Vec<Category>,
    pub limit: usize,
}

/// An answer's item: its record, and how it scored.
#[derive(Clone, Debug)]
pub struct Found {
    pub record: Record,
    pub score: f32,
}

/// How the person used an item: how many times they opened it from a search, and when last.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    pub count: u32,
    pub last: i64,
}

/// The index's fields.
#[derive(Clone, Copy)]
struct Fields {
    id: Field,
    source: Field,
    owner: Field,
    category: Field,
    title: Field,
    title_prefix: Field,
    exact: Field,
    keywords: Field,
    keywords_prefix: Field,
    tag: Field,
    content: Field,
    folder: Field,
    modified: Field,
    changed: Field,
    size: Field,
    record: Field,
}

const WORDS: &str = "oxyos_words";
const PREFIXES: &str = "oxyos_prefixes";
const EXACT: &str = "oxyos_exact";

fn schema() -> (Schema, Fields) {
    let mut builder = Schema::builder();
    let text = |tokenizer: &str, record: IndexRecordOption| {
        TextOptions::default().set_indexing_options(
            TextFieldIndexing::default()
                .set_tokenizer(tokenizer)
                .set_index_option(record),
        )
    };
    let positions = IndexRecordOption::WithFreqsAndPositions;
    let freqs = IndexRecordOption::WithFreqs;
    let fields = Fields {
        id: builder.add_text_field("id", STRING | FAST),
        source: builder.add_text_field("source", STRING),
        owner: builder.add_text_field("owner", STRING),
        category: builder.add_text_field("category", STRING),
        title: builder.add_text_field("title", text(WORDS, positions)),
        title_prefix: builder.add_text_field("title_prefix", text(PREFIXES, freqs)),
        exact: builder.add_text_field("exact", text(EXACT, IndexRecordOption::Basic)),
        keywords: builder.add_text_field("keywords", text(WORDS, positions)),
        keywords_prefix: builder.add_text_field("keywords_prefix", text(PREFIXES, freqs)),
        tag: builder.add_text_field("tag", text(EXACT, IndexRecordOption::Basic)),
        content: builder.add_text_field("content", text(WORDS, positions)),
        folder: builder.add_text_field("folder", STRING),
        modified: builder.add_i64_field("modified", INDEXED | FAST),
        changed: builder.add_i64_field("changed", FAST),
        size: builder.add_u64_field("size", FAST),
        record: builder.add_text_field("record", STORED),
    };
    (builder.build(), fields)
}

/// Words, lowercase and folded.
fn words_analyzer() -> TextAnalyzer {
    TextAnalyzer::builder(SimpleTokenizer::default())
        .filter(RemoveLongFilter::limit(LONGEST_WORD))
        .filter(LowerCaser)
        .filter(AsciiFoldingFilter)
        .build()
}

fn register(index: &tantivy::Index) {
    let tokenizers = index.tokenizers();
    tokenizers.register(WORDS, words_analyzer());
    tokenizers.register(
        PREFIXES,
        TextAnalyzer::builder(SimpleTokenizer::default())
            .filter(RemoveLongFilter::limit(LONGEST_WORD))
            .filter(LowerCaser)
            .filter(AsciiFoldingFilter)
            .filter(EdgePrefixes)
            .build(),
    );
    tokenizers.register(
        EXACT,
        TextAnalyzer::builder(RawTokenizer::default())
            .filter(LowerCaser)
            .filter(AsciiFoldingFilter)
            .build(),
    );
}

/// The words of `text` as the index folds them.
pub fn fold_words(text: &str) -> Vec<String> {
    let mut analyzer = words_analyzer();
    let mut stream = analyzer.token_stream(text);
    let mut words = Vec::new();
    while stream.advance() {
        words.push(stream.token().text.clone());
    }
    words
}

/// `text` whole, as the index folds an exact title or a tag.
pub fn fold_whole(text: &str) -> String {
    let mut analyzer = TextAnalyzer::builder(RawTokenizer::default())
        .filter(LowerCaser)
        .filter(AsciiFoldingFilter)
        .build();
    let mut stream = analyzer.token_stream(text.trim());
    let mut whole = String::new();
    while stream.advance() {
        whole.push_str(&stream.token().text);
    }
    whole
}

/// Every beginning of each word, one to [`LONGEST_PREFIX`] characters, at the word's position.
#[derive(Clone)]
struct EdgePrefixes;

impl TokenFilter for EdgePrefixes {
    type Tokenizer<T: Tokenizer> = EdgePrefixesFilter<T>;

    fn transform<T: Tokenizer>(self, tokenizer: T) -> EdgePrefixesFilter<T> {
        EdgePrefixesFilter { tokenizer }
    }
}

#[derive(Clone)]
struct EdgePrefixesFilter<T> {
    tokenizer: T,
}

impl<T: Tokenizer> Tokenizer for EdgePrefixesFilter<T> {
    type TokenStream<'a> = EdgePrefixesStream<T::TokenStream<'a>>;

    fn token_stream<'a>(&'a mut self, text: &'a str) -> Self::TokenStream<'a> {
        EdgePrefixesStream {
            tail: self.tokenizer.token_stream(text),
            word: String::new(),
            ends: Vec::new(),
            next: 0,
            token: Token::default(),
        }
    }
}

struct EdgePrefixesStream<S> {
    tail: S,
    /// The word whose beginnings are given.
    word: String,
    /// The byte length of each of its beginnings.
    ends: Vec<usize>,
    next: usize,
    token: Token,
}

impl<S: TokenStream> TokenStream for EdgePrefixesStream<S> {
    fn advance(&mut self) -> bool {
        while self.next >= self.ends.len() {
            if !self.tail.advance() {
                return false;
            }
            let token = self.tail.token();
            self.word.clone_from(&token.text);
            self.token.clone_from(token);
            self.ends = self
                .word
                .char_indices()
                .skip(1)
                .map(|(at, _)| at)
                .chain(std::iter::once(self.word.len()))
                .take(LONGEST_PREFIX)
                .collect();
            self.next = 0;
        }
        self.token.text.clear();
        self.token.text.push_str(&self.word[..self.ends[self.next]]);
        self.next += 1;
        true
    }

    fn token(&self) -> &Token {
        &self.token
    }

    fn token_mut(&mut self) -> &mut Token {
        &mut self.token
    }
}

/// Which of the cache's directories holds the index answering, and which a rebuild under way
/// (`index-N.json`, written whole: the one record of a rebuild's state on the disk). Without the
/// file, the index is `index-N`, as before there were rebuilds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Layout {
    /// The directory of the index answering.
    pub live: String,
    /// The directory of the index being rebuilt, until it takes the live one's place.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rebuild: Option<String>,
}

impl Layout {
    fn file(folder: &Path) -> PathBuf {
        folder.join(format!("index-{VERSION}.json"))
    }

    /// The layout of the index in `folder`.
    pub fn read(folder: &Path) -> Layout {
        std::fs::read(Layout::file(folder))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Layout>(&bytes).ok())
            .filter(|layout| {
                // Only this version's directories, by their names alone.
                let ours = |name: &str| generation(name).is_some();
                ours(&layout.live) && layout.rebuild.as_deref().is_none_or(ours)
            })
            .unwrap_or_else(|| Layout {
                live: format!("index-{VERSION}"),
                rebuild: None,
            })
    }

    /// Writes it whole, synced, then the folder's entry: after a crash the file is the old one or
    /// this one.
    fn write(&self, folder: &Path) -> std::io::Result<()> {
        use std::io::Write as _;
        let file = Layout::file(folder);
        let staged = file.with_extension("json.staged");
        {
            let mut out = std::fs::File::create(&staged)?;
            out.write_all(&serde_json::to_vec(self).expect("JSON"))?;
            out.sync_all()?;
        }
        std::fs::rename(&staged, &file)?;
        std::fs::File::open(folder)?.sync_all()
    }
}

/// A directory's generation: `index-N` is 0, `index-N.G` is G; none for another version's.
fn generation(name: &str) -> Option<u64> {
    let rest = name.strip_prefix(&format!("index-{VERSION}"))?;
    if rest.is_empty() {
        return Some(0);
    }
    rest.strip_prefix('.')?.parse().ok()
}

/// The index, open.
pub struct Index {
    index: tantivy::Index,
    fields: Fields,
    reader: IndexReader,
    /// What answers questions from it, shared with every thread that asks: a rebuilt index
    /// taking its place points it at its own reader.
    handle: Searcher,
    writer: Option<IndexWriter>,
    /// Changes not yet committed.
    uncommitted: usize,
    /// Its cache folder and directory's name (none in memory).
    place: Option<(PathBuf, String)>,
}

impl Index {
    /// Opens the index answering in `folder` (the directory its [`Layout`] names), making it if
    /// there is none or the one there cannot be read (another version's, or damaged: it is made
    /// again, as a cache is). Every other directory is removed (other versions', an index a
    /// rebuild replaced), but a rebuild's under way, which [`Index::rebuild`] goes on with.
    pub fn open(folder: &Path) -> tantivy::Result<Index> {
        let layout = Layout::read(folder);
        if let Ok(entries) = std::fs::read_dir(folder) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                let kept = name == layout.live.as_str()
                    || layout.rebuild.as_deref() == Some(&*name)
                    || entry.path() == Layout::file(folder);
                if name.starts_with("index-") && !kept {
                    let _ = if entry.path().is_dir() {
                        std::fs::remove_dir_all(entry.path())
                    } else {
                        std::fs::remove_file(entry.path())
                    };
                }
            }
        }
        Index::open_at(folder, &layout.live)
    }

    /// Opens or makes the index in `folder`'s directory `name`.
    fn open_at(folder: &Path, name: &str) -> tantivy::Result<Index> {
        let directory = folder.join(name);
        let (schema, fields) = schema();
        let open = || -> tantivy::Result<tantivy::Index> {
            std::fs::create_dir_all(&directory)?;
            let mmap = tantivy::directory::MmapDirectory::open(&directory)?;
            let settings = tantivy::IndexSettings {
                // One thread fewer: the stores are compressed on the writer's own.
                docstore_compress_dedicated_thread: false,
                ..Default::default()
            };
            tantivy::Index::builder()
                .schema(schema.clone())
                .settings(settings)
                .open_or_create(mmap)
        };
        let index = match open() {
            Ok(index) if index.schema() == schema => index,
            _ => {
                let _ = std::fs::remove_dir_all(&directory);
                open()?
            }
        };
        let mut index = Index::with(index, fields)?;
        index.place = Some((folder.to_path_buf(), name.to_string()));
        Ok(index)
    }

    /// Whether a rebuild was under way in `folder` when the service last stopped.
    pub fn rebuild_under_way(folder: &Path) -> bool {
        Layout::read(folder).rebuild.is_some()
    }

    /// The index a rebuild writes, beside the one answering: the one under way when `resume` and
    /// there is one (its committed items are not read again), else a new, empty one, named in the
    /// layout before anything is written to it.
    pub fn rebuild(folder: &Path, resume: bool) -> tantivy::Result<Index> {
        let mut layout = Layout::read(folder);
        if let (true, Some(name)) = (resume, layout.rebuild.clone()) {
            return Index::open_at(folder, &name);
        }
        if let Some(abandoned) = layout.rebuild.take() {
            let _ = std::fs::remove_dir_all(folder.join(abandoned));
        }
        let next = generation(&layout.live).unwrap_or(0) + 1;
        let name = format!("index-{VERSION}.{next}");
        let _ = std::fs::remove_dir_all(folder.join(&name));
        let index = Index::open_at(folder, &name)?;
        layout.rebuild = Some(name);
        layout.write(folder)?;
        Ok(index)
    }

    /// The rebuilt index takes the place of `live`, the one answering: everything it holds
    /// committed and its writer closed, the layout names it (the one step that changes which
    /// index is the person's, written whole), every handle that answers questions is pointed at
    /// it, and the old one's directory is removed (the mapped segments of a question under way
    /// stay readable until it ends). Returns the index now answering: `self`, or `live` if the
    /// layout could not be written (the rebuild then goes on at the next start).
    pub fn promote(mut self, live: Index) -> (Index, tantivy::Result<()>) {
        let promoted = (|| -> tantivy::Result<()> {
            self.commit(true)?;
            let (folder, name) = self.place.clone().ok_or_else(|| {
                tantivy::TantivyError::InvalidArgument("an index in memory".into())
            })?;
            Layout {
                live: name,
                rebuild: None,
            }
            .write(&folder)?;
            Ok(())
        })();
        if promoted.is_err() {
            return (live, promoted);
        }
        live.handle.point_at(&self.reader);
        self.handle = live.handle.clone();
        let old = live.place.clone();
        drop(live);
        if let Some((folder, name)) = old {
            let _ = std::fs::remove_dir_all(folder.join(name));
        }
        (self, Ok(()))
    }

    /// An index in memory (tests).
    pub fn in_memory() -> tantivy::Result<Index> {
        let (schema, fields) = schema();
        Index::with(tantivy::Index::create_in_ram(schema), fields)
    }

    fn with(index: tantivy::Index, fields: Fields) -> tantivy::Result<Index> {
        register(&index);
        // Reloaded by the service after each commit: a reader that follows the directory itself
        // watches it on a clock.
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .num_warming_threads(0)
            .try_into()?;
        let handle = Searcher {
            reader: Arc::new(RwLock::new(reader.clone())),
            fields,
        };
        Ok(Index {
            index,
            fields,
            reader,
            handle,
            writer: None,
            uncommitted: 0,
            place: None,
        })
    }

    /// The reader, for a second handle on another thread (the service's questions).
    pub fn reader(&self) -> IndexReader {
        self.reader.clone()
    }

    /// A handle that answers questions, on the reader's segments as last reloaded; shared, so
    /// that it answers from a rebuilt index once that takes this one's place.
    pub fn searcher_handle(&self) -> Searcher {
        self.handle.clone()
    }

    fn writer(&mut self) -> tantivy::Result<&mut IndexWriter> {
        if self.writer.is_none() {
            self.writer = Some(self.index.writer_with_num_threads(1, WRITER_BUDGET)?);
        }
        Ok(self.writer.as_mut().expect("opened"))
    }

    /// Adds `record` (replacing any item of its id) with its `text`.
    pub fn put(&mut self, record: &Record, text: Option<&str>) -> tantivy::Result<()> {
        let fields = self.fields;
        let document = document(fields, record, text);
        let writer = self.writer()?;
        // (`document` keeps a donated item's text in its record: nothing else can give it again.)
        writer.delete_term(Term::from_field_text(fields.id, &record.id));
        writer.add_document(document)?;
        self.uncommitted += 1;
        Ok(())
    }

    /// Removes the item `id`.
    pub fn remove(&mut self, id: &str) -> tantivy::Result<()> {
        let term = Term::from_field_text(self.fields.id, id);
        self.writer()?.delete_term(term);
        self.uncommitted += 1;
        Ok(())
    }

    /// Removes every item inside the folder `folder` (not the folder itself).
    pub fn remove_inside(&mut self, folder: &Path) -> tantivy::Result<()> {
        let term = Term::from_field_text(self.fields.folder, &folder.to_string_lossy());
        self.writer()?.delete_term(term);
        self.uncommitted += 1;
        Ok(())
    }

    /// Removes every item of `source` (and of `owner`, if given).
    pub fn remove_source(&mut self, source: &Source, owner: Option<&str>) -> tantivy::Result<()> {
        let fields = self.fields;
        let mut clauses: Vec<(Occur, Box<dyn Query>)> =
            vec![(Occur::Must, term_query(fields.source, source.name()))];
        if let Some(owner) = owner {
            clauses.push((Occur::Must, term_query(fields.owner, owner)));
        }
        self.writer()?
            .delete_query(Box::new(BooleanQuery::new(clauses)))?;
        self.uncommitted += 1;
        Ok(())
    }

    /// Changes not yet committed.
    pub fn uncommitted(&self) -> usize {
        self.uncommitted
    }

    /// Writes what changed, and reloads the reader. With `close`, the writer is closed after: its
    /// threads end and its memory is given back until something changes again (the idle rule).
    pub fn commit(&mut self, close: bool) -> tantivy::Result<()> {
        if let Some(writer) = self.writer.as_mut() {
            if self.uncommitted > 0 {
                writer.commit()?;
                self.uncommitted = 0;
            }
        }
        if close {
            if let Some(writer) = self.writer.take() {
                writer.wait_merging_threads()?;
            }
        }
        self.reader.reload()
    }

    /// Every file and folder the index holds ([`Searcher::files`]).
    pub fn files(&self) -> tantivy::Result<HashMap<String, (i64, u64)>> {
        self.searcher_handle().files()
    }

    /// The record of `id`, if the index holds it.
    pub fn record(&self, id: &str) -> Option<Record> {
        self.searcher_handle().record(id)
    }

    /// How many items it holds, and its size on the disk.
    pub fn count(&self) -> u64 {
        self.reader.searcher().num_docs()
    }
}

fn term_query(field: Field, text: &str) -> Box<dyn Query> {
    Box::new(TermQuery::new(
        Term::from_field_text(field, text),
        IndexRecordOption::Basic,
    ))
}

/// Every folder above `path`, the closest first.
fn folders(path: &Path) -> impl Iterator<Item = &Path> {
    path.ancestors()
        .skip(1)
        .filter(|folder| folder.parent().is_some())
}

fn document(fields: Fields, record: &Record, text: Option<&str>) -> TantivyDocument {
    let mut document = TantivyDocument::default();
    document.add_text(fields.id, &record.id);
    document.add_text(fields.source, record.source.name());
    if let Some(owner) = &record.owner {
        document.add_text(fields.owner, owner);
    }
    document.add_text(fields.category, record.category.name());
    let mut titles: Vec<&String> = record.titles.values().collect();
    titles.dedup();
    for title in titles {
        document.add_text(fields.title, title);
        document.add_text(fields.title_prefix, title);
        document.add_text(fields.exact, title);
        // A file's name without its extension is its exact title too (`report` is `report.pdf`).
        if record.source == Source::File && record.category != Category::Folder {
            if let Some((stem, _)) = title.rsplit_once('.').filter(|(stem, _)| !stem.is_empty()) {
                document.add_text(fields.exact, stem);
            }
        }
    }
    for keyword in record.keywords.iter().chain(&record.tags) {
        document.add_text(fields.keywords, keyword);
        document.add_text(fields.keywords_prefix, keyword);
    }
    for tag in &record.tags {
        document.add_text(fields.tag, tag);
    }
    if let Some(text) = text {
        document.add_text(fields.content, text);
    }
    if let Some(path) = &record.path {
        for folder in folders(path) {
            document.add_text(fields.folder, folder.to_string_lossy());
        }
    }
    document.add_i64(fields.modified, record.modified.unwrap_or(0));
    document.add_i64(fields.changed, record.changed.unwrap_or(0));
    document.add_u64(fields.size, record.size.unwrap_or(0));
    let mut stored = serde_json::to_value(record).expect("a record is JSON");
    if let (Source::Donated, Some(text), Some(object)) =
        (&record.source, text, stored.as_object_mut())
    {
        object.insert(DONATED_TEXT.into(), text.into());
    }
    document.add_text(fields.record, stored.to_string());
    document
}

/// The key of a donated item's text in its stored record.
const DONATED_TEXT: &str = "donatedText";

/// What answers questions: a reader's handle, on any thread, its clones one handle (pointed at a
/// rebuilt index's reader when that takes the old one's place).
#[derive(Clone)]
pub struct Searcher {
    reader: Arc<RwLock<IndexReader>>,
    fields: Fields,
}

impl Searcher {
    fn tantivy(&self) -> tantivy::Searcher {
        self.reader.read().expect("lock").searcher()
    }

    fn point_at(&self, reader: &IndexReader) {
        *self.reader.write().expect("lock") = reader.clone();
    }

    /// Takes the index as last committed.
    pub fn reload(&self) -> tantivy::Result<()> {
        self.reader.read().expect("lock").reload()
    }

    /// How many items it holds.
    pub fn count(&self) -> u64 {
        self.tantivy().num_docs()
    }

    /// Its size on the disk, in bytes.
    pub fn disk_size(&self) -> u64 {
        self.tantivy()
            .segment_readers()
            .iter()
            .filter_map(|segment| segment.space_usage().ok())
            .map(|usage| usage.total().get_bytes())
            .sum()
    }

    /// Every file and folder the index holds: its path's id, the time it changed and its size.
    pub fn files(&self) -> tantivy::Result<HashMap<String, (i64, u64)>> {
        let searcher = self.tantivy();
        let mut files = HashMap::new();
        let query = term_query(self.fields.source, Source::File.name());
        let weight = query.weight(tantivy::query::EnableScoring::disabled_from_searcher(
            &searcher,
        ))?;
        for segment in searcher.segment_readers() {
            let fast = segment.fast_fields();
            let ids = fast.str("id")?;
            let changed = fast.i64("changed")?;
            let sizes = fast.u64("size")?;
            let Some(ids) = ids else {
                continue;
            };
            let mut text = String::new();
            let alive = segment.alive_bitset();
            weight.for_each_no_score(segment, &mut |documents| {
                for &document in documents {
                    // Deleted, its segment not yet merged.
                    if alive.is_some_and(|alive| !alive.is_alive(document)) {
                        continue;
                    }
                    let Some(ordinal) = ids.term_ords(document).next() else {
                        continue;
                    };
                    if ids.ord_to_str(ordinal, &mut text).unwrap_or(false) {
                        let changed = changed.first(document).unwrap_or(0);
                        let size = sizes.first(document).unwrap_or(0);
                        files.insert(text.clone(), (changed, size));
                    }
                }
            })?;
        }
        Ok(files)
    }

    /// The record of `id`.
    pub fn record(&self, id: &str) -> Option<Record> {
        let searcher = self.tantivy();
        let found = searcher
            .search(
                &TermQuery::new(
                    Term::from_field_text(self.fields.id, id),
                    IndexRecordOption::Basic,
                ),
                &TopDocs::with_limit(1).order_by_score(),
            )
            .ok()?;
        let (_, address) = found.first()?;
        self.fetch(&searcher, *address)
    }

    fn fetch(&self, searcher: &tantivy::Searcher, address: tantivy::DocAddress) -> Option<Record> {
        let document: TantivyDocument = searcher.doc(address).ok()?;
        let text = document.get_first(self.fields.record)?.as_str()?;
        serde_json::from_str(text).ok()
    }

    /// Every item that is not a file or folder (the applications, the items packages declare and
    /// those applications donated), with a donated item's text: what a rebuild cannot read again
    /// from the disk, carried over to the new index.
    pub fn given(&self) -> tantivy::Result<Vec<(Record, Option<String>)>> {
        let searcher = self.tantivy();
        let query = BooleanQuery::new(vec![
            (Occur::Must, Box::new(AllQuery) as Box<dyn Query>),
            (
                Occur::MustNot,
                term_query(self.fields.source, Source::File.name()),
            ),
        ]);
        let mut given = Vec::new();
        for address in searcher.search(&query, &DocSetCollector)? {
            let document: TantivyDocument = searcher.doc(address)?;
            let Some(stored) = document
                .get_first(self.fields.record)
                .and_then(|value| value.as_str())
                .and_then(|text| serde_json::from_str::<serde_json::Value>(text).ok())
            else {
                continue;
            };
            let text = stored[DONATED_TEXT].as_str().map(String::from);
            if let Ok(record) = serde_json::from_value::<Record>(stored) {
                given.push((record, text));
            }
        }
        Ok(given)
    }

    /// The items answering `question` for `viewer`, best first, weighed by `usage` as of `now`
    /// (milliseconds), with the top hit's index among them.
    pub fn search(
        &self,
        question: &Question,
        viewer: &Viewer,
        usage: &HashMap<String, Usage>,
        now: i64,
    ) -> tantivy::Result<(Vec<Found>, Option<usize>)> {
        let parsed = &question.parsed;
        if parsed.is_empty() || parsed.impossible {
            return Ok((Vec::new(), None));
        }
        let fields = self.fields;
        let mut clauses: Vec<(Occur, Box<dyn Query>)> = Vec::new();
        let words: Vec<String> = parsed
            .words
            .iter()
            .flat_map(|word| fold_words(word))
            .collect();
        let last = words.len().checked_sub(1);
        for (at, word) in words.iter().enumerate() {
            let mut either: Vec<(Occur, Box<dyn Query>)> = vec![
                boosted(fields.title_prefix, word, 3.0),
                boosted(fields.title, word, 2.0),
                boosted(fields.keywords_prefix, word, 1.5),
                boosted(fields.content, word, 1.0),
            ];
            // The word being typed, a word's beginning in the text too.
            if Some(at) == last && word.chars().count() >= 2 {
                let mut prefix = PhrasePrefixQuery::new_with_offset(vec![(
                    0,
                    Term::from_field_text(fields.content, word),
                )]);
                prefix.set_max_expansions(64);
                either.push((
                    Occur::Should,
                    Box::new(BoostQuery::new(Box::new(prefix), 0.5)),
                ));
            }
            clauses.push((Occur::Must, Box::new(BooleanQuery::new(either))));
        }
        for phrase in &parsed.phrases {
            let terms = fold_words(phrase);
            match terms.len() {
                0 => {}
                1 => clauses.push((
                    Occur::Must,
                    Box::new(BooleanQuery::new(vec![
                        boosted(fields.title, &terms[0], 3.0),
                        boosted(fields.keywords, &terms[0], 1.5),
                        boosted(fields.content, &terms[0], 1.0),
                    ])),
                )),
                _ => {
                    let phrase_in = |field: Field, boost: f32| -> (Occur, Box<dyn Query>) {
                        let terms = terms
                            .iter()
                            .map(|term| Term::from_field_text(field, term))
                            .collect();
                        (
                            Occur::Should,
                            Box::new(BoostQuery::new(Box::new(PhraseQuery::new(terms)), boost)),
                        )
                    };
                    clauses.push((
                        Occur::Must,
                        Box::new(BooleanQuery::new(vec![
                            phrase_in(fields.title, 3.0),
                            phrase_in(fields.content, 1.0),
                        ])),
                    ));
                }
            }
        }
        for name in &parsed.names {
            for word in fold_words(name) {
                clauses.push((Occur::Must, boosted(fields.title_prefix, &word, 3.0).1));
            }
        }
        // The whole text typed is the title: first.
        if !parsed.words.is_empty() {
            let whole = fold_whole(&parsed.words.join(" "));
            if !whole.is_empty() {
                clauses.push(boosted(fields.exact, &whole, 10.0));
            }
        }
        // Filters, which score nothing.
        let mut filters: Vec<(Occur, Box<dyn Query>)> = Vec::new();
        let mut categories: Vec<Category> = parsed.categories.clone();
        if !question.categories.is_empty() {
            categories = if categories.is_empty() {
                question.categories.clone()
            } else {
                categories
                    .into_iter()
                    .filter(|category| question.categories.contains(category))
                    .collect()
            };
            if categories.is_empty() {
                return Ok((Vec::new(), None));
            }
        }
        if !categories.is_empty() {
            filters.push((
                Occur::Must,
                any_of(fields.category, categories.iter().map(|c| c.name())),
            ));
        }
        for category in &viewer.hidden {
            filters.push((Occur::MustNot, term_query(fields.category, category.name())));
        }
        for tag in &parsed.tags {
            filters.push((Occur::Must, term_query(fields.tag, &fold_whole(tag))));
        }
        if let Some(Span { from, to }) = parsed.dates {
            filters.push((
                Occur::Must,
                Box::new(RangeQuery::new(
                    Bound::Included(Term::from_field_i64(fields.modified, from)),
                    Bound::Excluded(Term::from_field_i64(fields.modified, to)),
                )),
            ));
        }
        if !question.within.is_empty() {
            let inside: Vec<String> = question
                .within
                .iter()
                .map(|folder| folder.to_string_lossy().into_owned())
                .collect();
            filters.push((
                Occur::Must,
                any_of(fields.folder, inside.iter().map(String::as_str)),
            ));
        }
        if !viewer.shell {
            let mut seen = vec![(
                Occur::Should,
                term_query(fields.source, Source::File.name()),
            )];
            if let Some(application) = &viewer.application {
                seen.push((Occur::Should, term_query(fields.owner, application)));
            }
            filters.push((Occur::Must, Box::new(BooleanQuery::new(seen))));
        }
        let scored: Box<dyn Query> = if clauses.is_empty() {
            Box::new(AllQuery)
        } else {
            Box::new(BooleanQuery::new(clauses))
        };
        let mut all = vec![(Occur::Must, scored)];
        all.extend(filters);
        let query = BooleanQuery::new(all);
        let searcher = self.tantivy();
        let candidates = searcher.search(
            &query,
            &TopDocs::with_limit(CANDIDATES.max(question.limit)).order_by_score(),
        )?;
        let mut found: Vec<Found> = candidates
            .into_iter()
            .filter_map(|(score, address)| {
                let record = self.fetch(&searcher, address)?;
                let score = score * weight(&record, usage.get(&record.id), now);
                Some(Found { record, score })
            })
            .collect();
        found.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then_with(|| a.record.id.cmp(&b.record.id))
        });
        found.truncate(question.limit);
        let top = found.first().and_then(|first| {
            let typed: Vec<String> = words
                .iter()
                .cloned()
                .chain(parsed.names.iter().flat_map(|name| fold_words(name)))
                .collect();
            if typed.is_empty() {
                return None;
            }
            // Its title's words, and the words it is known by (an application's keywords, a
            // Settings page's declared terms, a file's tags).
            let title_words: Vec<String> = first
                .record
                .titles
                .values()
                .chain(&first.record.keywords)
                .chain(&first.record.tags)
                .flat_map(|title| fold_words(title))
                .collect();
            typed
                .iter()
                .all(|word| {
                    title_words
                        .iter()
                        .any(|title| title.starts_with(word.as_str()))
                })
                .then_some(0)
        });
        Ok((found, top))
    }

    /// A snippet of `text` around the first of `parsed`'s words it holds, with the words
    /// highlighted nowhere (the panel shows it plainly): about 160 characters.
    pub fn snippet(text: &str, parsed: &Parsed) -> Option<String> {
        let wanted: Vec<String> = parsed
            .words
            .iter()
            .chain(&parsed.phrases)
            .flat_map(|word| fold_words(word))
            .collect();
        if wanted.is_empty() {
            return None;
        }
        // Each word of the text with where it is, folded as the index folds it.
        let mut analyzer = words_analyzer();
        let mut stream = analyzer.token_stream(text);
        let mut hit = None;
        while stream.advance() {
            let token = stream.token();
            if wanted
                .iter()
                .any(|word| token.text.starts_with(word.as_str()))
            {
                hit = Some((token.offset_from, token.offset_to));
                break;
            }
        }
        let (from, _) = hit?;
        let mut start = from.saturating_sub(60);
        while !text.is_char_boundary(start) {
            start -= 1;
        }
        // From a word's beginning.
        if start > 0 {
            if let Some(space) = text[start..from].find(char::is_whitespace) {
                start += space + 1;
            }
        }
        let mut end = (start + 160).min(text.len());
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        if end < text.len() {
            if let Some(space) = text[from..end].rfind(char::is_whitespace) {
                end = from + space;
            }
        }
        let snippet: String = text[start..end]
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        Some(format!(
            "{}{}{}",
            if start > 0 { "…" } else { "" },
            snippet,
            if end < text.len() { "…" } else { "" }
        ))
    }
}

fn boosted(field: Field, word: &str, boost: f32) -> (Occur, Box<dyn Query>) {
    (
        Occur::Should,
        Box::new(BoostQuery::new(
            Box::new(TermQuery::new(
                Term::from_field_text(field, word),
                IndexRecordOption::WithFreqs,
            )),
            boost,
        )),
    )
}

fn any_of<'a>(field: Field, values: impl Iterator<Item = &'a str>) -> Box<dyn Query> {
    Box::new(BooleanQuery::new(
        values
            .map(|value| (Occur::Should, term_query(field, value)))
            .collect(),
    ))
}

/// What weighs a candidate beyond its words (decision 5): its category, how recently it changed
/// or was used, and how often the person opened it from a search.
pub fn weight(record: &Record, usage: Option<&Usage>, now: i64) -> f32 {
    const DAY: f64 = 86_400_000.0;
    let category = match record.category {
        Category::Application => 1.6,
        Category::Settings => 1.2,
        Category::Folder => 1.1,
        _ => 1.0,
    };
    let touched = record
        .modified
        .into_iter()
        .chain(usage.map(|usage| usage.last))
        .max();
    let recency = match (record.source.clone(), touched) {
        (Source::File | Source::Donated, Some(touched)) => {
            let days = ((now - touched).max(0) as f64) / DAY;
            1.0 + 0.5 * 0.5f64.powf(days / 30.0)
        }
        _ => 1.0,
    };
    let used = usage.map_or(1.0, |usage| {
        let days = ((now - usage.last).max(0) as f64) / DAY;
        1.0 + 0.6 * f64::from(usage.count).ln_1p() * (0.5 + 0.5 * 0.5f64.powf(days / 60.0))
    });
    (category * recency * used) as f32
}
