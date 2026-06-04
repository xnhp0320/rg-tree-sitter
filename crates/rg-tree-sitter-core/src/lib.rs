pub mod cache;
pub mod engine;
pub mod filter;
pub mod languages;
pub mod prefilter;
pub mod searcher;

pub use filter::{AstFilter, SemanticMatch};
pub use languages::{guess_language_from_path, LanguageId, LanguageRules, SymbolKind};
pub use searcher::{parse_external_matches, search_symbol, TextMatch};

use std::path::Path;

/// High-level API: search for symbol definitions.
pub fn find_definitions(
    symbol: &str,
    dir: &Path,
    lang: LanguageId,
) -> anyhow::Result<Vec<SemanticMatch>> {
    let extensions = lang.extensions();
    let matches = searcher::search_symbol(symbol, dir, extensions)?;
    let matches = prefilter::quick_filter(&matches);
    let mut filter = AstFilter::new(lang)?;
    Ok(filter.filter_definitions(&matches))
}

/// High-level API: search for symbol references.
pub fn find_references(
    symbol: &str,
    dir: &Path,
    lang: LanguageId,
) -> anyhow::Result<Vec<SemanticMatch>> {
    let extensions = lang.extensions();
    let matches = searcher::search_symbol(symbol, dir, extensions)?;
    let matches = prefilter::quick_filter(&matches);
    let mut filter = AstFilter::new(lang)?;
    Ok(filter.filter_references(&matches))
}

/// High-level API: filter external rg output for definitions.
pub fn filter_definitions_from_input<R: std::io::Read>(
    input: R,
    lang: LanguageId,
) -> anyhow::Result<Vec<SemanticMatch>> {
    let matches = searcher::parse_external_matches(input)?;
    let matches = prefilter::quick_filter(&matches);
    let mut filter = AstFilter::new(lang)?;
    Ok(filter.filter_definitions(&matches))
}
