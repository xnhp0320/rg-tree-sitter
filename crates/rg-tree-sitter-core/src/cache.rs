use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Simple LRU-like cache for parsed ASTs.
pub struct AstCache {
    inner: Mutex<CacheInner>,
    capacity: usize,
}

struct CacheEntry {
    tree: tree_sitter::Tree,
    source: String,
    mtime: std::time::SystemTime,
}

struct CacheInner {
    data: HashMap<PathBuf, CacheEntry>,
    order: Vec<PathBuf>, // Most recently used at the end
}

impl AstCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(CacheInner {
                data: HashMap::new(),
                order: Vec::new(),
            }),
            capacity,
        }
    }

    pub fn get(&self, path: &Path) -> Option<(tree_sitter::Tree, String)> {
        let mut inner = self.inner.lock().unwrap();
        let entry = inner.data.get(path)?;

        // Check mtime
        let current_mtime = std::fs::metadata(path).ok()?.modified().ok()?;
        if current_mtime != entry.mtime {
            return None;
        }

        // Clone data first before modifying order
        let tree = entry.tree.clone();
        let source = entry.source.clone();

        // Move to end (most recently used)
        if let Some(pos) = inner.order.iter().position(|p| p == path) {
            let p = inner.order.remove(pos);
            inner.order.push(p);
        }

        Some((tree, source))
    }

    pub fn insert(&self, path: PathBuf, tree: tree_sitter::Tree, source: String) {
        let mut inner = self.inner.lock().unwrap();

        let mtime = std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .unwrap_or_else(|_| std::time::SystemTime::now());

        // Remove old entry if exists
        if inner.data.contains_key(&path) {
            inner.order.retain(|p| p != &path);
        }

        // Evict if over capacity
        while inner.order.len() >= self.capacity {
            if let Some(old) = inner.order.first().cloned() {
                inner.order.remove(0);
                inner.data.remove(&old);
            } else {
                break;
            }
        }

        inner.order.push(path.clone());
        inner.data.insert(
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
        inner.order.retain(|p| p != path);
        inner.data.remove(path);
    }

    pub fn clear(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.data.clear();
        inner.order.clear();
    }

    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().order.len()
    }
}

/// Shared cache reference for use across async boundaries.
pub type SharedAstCache = Arc<AstCache>;

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
