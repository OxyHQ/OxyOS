//! OxyOS's search (docs/adr/0051-search.mdx): the session's index of the person's files,
//! applications, Settings' pages and what applications donate, and the questions asked of it, as a
//! Mac's Spotlight answers them.
//!
//! - [`category`]: what kind of thing an item is (its section in the results, `kind:`).
//! - [`query`]: the query language: words, phrases, typed filters and a field's tokens.
//! - [`extract`]: the text of a file, by proven readers (UTF-8 text, ZIP and XML for office
//!   documents, MuPDF for PDFs).
//! - [`index`]: tantivy's index, its fields and the ranking.

pub mod category;
pub mod extract;
pub mod index;
pub mod query;

#[cfg(test)]
mod index_tests;
