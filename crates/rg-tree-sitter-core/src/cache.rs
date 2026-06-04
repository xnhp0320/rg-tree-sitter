use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Simple LRU cache for parsed ASTs, backed by the `lru` crate.
pub struct AstCache {
    inner: Mutex<lru::LruCache<PathBuf, CacheEntry>>,
}

struct CacheEntry {
    tree: tree_sitter::Tree,
    source: String,
    mtime: std::time::SystemTime,
}

impl AstCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(lru::LruCache::new(
                std::num::NonZeroUsize::new(capacity).unwrap_or(
                    std::num::NonZeroUsize::new(1).unwrap(),
                ),
            )),
        }
    }

    pub fn get(&self, path: &Path) -> Option<(tree_sitter::Tree, String)> {
        let mut inner = self.inner.lock().unwrap();
        let entry = inner.get(path)?;

        // Check mtime
        let current_mtime = std::fs::metadata(path).ok()?.modified().ok()?;
        if current_mtime != entry.mtime {
            // Stale entry — remove it
            inner.pop(path);
            return None;
        }

        Some((entry.tree.clone(), entry.source.clone()))
    }

    pub fn insert(&self, path: PathBuf, tree: tree_sitter::Tree, source: String) {
        let mut inner = self.inner.lock().unwrap();

        let mtime = std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .unwrap_or_else(|_| std::time::SystemTime::now());

        inner.put(
            path,
            CacheEntry {
                tree,
                source,
                mtime,
            },
        );
    }

    pub fn remove(&self, path: &Path) {
        let mut inner = self.inner.lock().unwrap();
        inner.pop(path);
    }

    pub fn clear(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.clear();
    }

    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().len()
    }
}

/// Shared cache reference for use across async boundaries.
pub type SharedAstCache = std::sync::Arc<AstCache>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_basic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.c");
        std::fs::write(&path, "void foo() {}\n").unwrap();

        let cache = AstCache::new(10);
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&tree_sitter_c::LANGUAGE.into()).unwrap();
        let source = std::fs::read_to_string(&path).unwrap();
        let tree = parser.parse(&source, None).unwrap();

        assert_eq!(cache.len(), 0);
        cache.insert(path.clone(), tree, source);
        assert_eq!(cache.len(), 1);

        let result = cache.get(&path);
        assert!(result.is_some());
    }

    #[test]
    fn test_cache_lru_eviction() {
        let dir = tempfile::tempdir().unwrap();
        let cache = AstCache::new(2);
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&tree_sitter_c::LANGUAGE.into()).unwrap();

        let mut paths = Vec::new();
        for i in 0..3 {
            let path = dir.path().join(format!("test{}.c", i));
            std::fs::write(&path, format!("void foo{}() {{}}\n", i)).unwrap();
            paths.push(path);
        }

        for path in &paths {
            let source = std::fs::read_to_string(path).unwrap();
            let tree = parser.parse(&source, None).unwrap();
            cache.insert(path.clone(), tree, source);
        }

        assert_eq!(cache.len(), 2);
        // The first entry should have been evicted
        assert!(cache.get(&paths[0]).is_none());
        // The last two should still be present
        assert!(cache.get(&paths[1]).is_some());
        assert!(cache.get(&paths[2]).is_some());
    }

    #[test]
    fn test_cache_mtime_check() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.c");
        std::fs::write(&path, "void foo() {}\n").unwrap();

        let cache = AstCache::new(10);
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&tree_sitter_c::LANGUAGE.into()).unwrap();
        let source = std::fs::read_to_string(&path).unwrap();
        let tree = parser.parse(&source, None).unwrap();

        cache.insert(path.clone(), tree, source);
        assert!(cache.get(&path).is_some());

        // Modify the file
        std::thread::sleep(std::time::Duration::from_millis(50));
        std::fs::write(&path, "void bar() {}\n").unwrap();

        // Cache should detect mtime change
        assert!(cache.get(&path).is_none());
    }

    #[test]
    fn test_cache_clear() {
        let cache = AstCache::new(10);
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&tree_sitter_c::LANGUAGE.into()).unwrap();
        let tree = parser.parse("void foo() {}", None).unwrap();

        cache.insert(PathBuf::from("/tmp/a.c"), tree.clone(), "void foo() {}".to_string());
        cache.insert(PathBuf::from("/tmp/b.c"), tree, "void bar() {}".to_string());
        assert_eq!(cache.len(), 2);

        cache.clear();
        assert_eq!(cache.len(), 0);
    }
}
