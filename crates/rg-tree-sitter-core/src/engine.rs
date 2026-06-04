use crate::{
    cache::AstCache,
    filter::AstFilter,
    languages::LanguageId,
    searcher::search_symbol,
    SemanticMatch,
};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;

/// Shared search engine with LRU AST cache, used by the daemon.
pub struct SearchEngine {
    cache: Arc<AstCache>,
    dirty_files: Mutex<HashSet<PathBuf>>,
}

impl SearchEngine {
    pub fn new(cache_capacity: usize) -> Self {
        Self {
            cache: Arc::new(AstCache::new(cache_capacity)),
            dirty_files: Mutex::new(HashSet::new()),
        }
    }

    pub fn cache(&self) -> Arc<AstCache> {
        Arc::clone(&self.cache)
    }

    /// Mark a file as dirty and evict it from cache immediately.
    pub fn mark_dirty(&self, path: PathBuf) {
        self.cache.remove(&path);
        self.dirty_files.lock().unwrap().insert(path);
    }

    pub fn find_definitions(
        &self,
        symbol: &str,
        dir: &Path,
        lang: LanguageId,
    ) -> anyhow::Result<Vec<SemanticMatch>> {
        let extensions = lang.extensions();
        let matches = search_symbol(symbol, dir, extensions)?;
        let matches = crate::prefilter::quick_filter(&matches);
        let mut filter = AstFilter::new_with_cache(lang, Some(self.cache()))?;
        Ok(filter.filter_definitions(&matches))
    }

    pub fn find_references(
        &self,
        symbol: &str,
        dir: &Path,
        lang: LanguageId,
    ) -> anyhow::Result<Vec<SemanticMatch>> {
        let extensions = lang.extensions();
        let matches = search_symbol(symbol, dir, extensions)?;
        let matches = crate::prefilter::quick_filter(&matches);
        let mut filter = AstFilter::new_with_cache(lang, Some(self.cache()))?;
        Ok(filter.filter_references(&matches))
    }

    pub fn clear_cache(&self) {
        self.cache.clear();
        self.dirty_files.lock().unwrap().clear();
    }

    pub fn cache_len(&self) -> usize {
        self.cache.len()
    }
}
