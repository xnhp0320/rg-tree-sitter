use crate::cache::AstCache;
use crate::languages::{LanguageId, LanguageRules, SymbolKind};
use crate::searcher::TextMatch;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

/// A match after tree-sitter semantic classification.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SemanticMatch {
    pub path: String,
    pub line: usize,
    pub column: usize,
    pub text: String,
    pub kind: SymbolKind,
}

/// Cache entry for parsed AST.
struct ParsedFile {
    tree: tree_sitter::Tree,
    source: String,
    mtime: std::time::SystemTime,
}

/// Filter text matches using tree-sitter AST semantics.
pub struct AstFilter {
    parser: tree_sitter::Parser,
    lang: LanguageId,
    rules: LanguageRules,
    // Simple in-memory cache for this filtering session
    local_cache: HashMap<std::path::PathBuf, ParsedFile>,
    // Optional shared daemon cache
    shared_cache: Option<Arc<AstCache>>,
}

impl AstFilter {
    pub fn new(lang: LanguageId) -> anyhow::Result<Self> {
        Self::new_with_cache(lang, None)
    }

    pub fn new_with_cache(lang: LanguageId, shared_cache: Option<Arc<AstCache>>) -> anyhow::Result<Self> {
        let mut parser = tree_sitter::Parser::new();
        let ts_lang = lang.to_tree_sitter_language();
        parser.set_language(&ts_lang)?;
        let rules = LanguageRules::for_language(lang);
        Ok(Self {
            parser,
            lang,
            rules,
            local_cache: HashMap::new(),
            shared_cache,
        })
    }

    pub fn filter_definitions(&mut self, matches: &[TextMatch]) -> Vec<SemanticMatch> {
        self.filter(matches, SymbolKind::Definition)
    }

    pub fn filter_calls(&mut self, matches: &[TextMatch]) -> Vec<SemanticMatch> {
        self.filter(matches, SymbolKind::Call)
    }

    pub fn filter_references(&mut self, matches: &[TextMatch]) -> Vec<SemanticMatch> {
        self.filter(matches, SymbolKind::Reference)
    }

    fn filter(&mut self, matches: &[TextMatch], target_kind: SymbolKind) -> Vec<SemanticMatch> {
        let mut result = Vec::new();
        let rules = self.rules; // Copy rules out to avoid borrow issues

        for m in matches {
            let path = &m.path;
            let parsed = match self.get_or_parse(path) {
                Some(p) => p,
                None => continue,
            };

            // tree-sitter uses 0-based row/col
            let row = (m.line.saturating_sub(1)) as usize;
            let col = (m.column.saturating_sub(1)) as usize;

            let node = match parsed.tree.root_node().descendant_for_point_range(
                tree_sitter::Point::new(row, col),
                tree_sitter::Point::new(row, col + 1),
            ) {
                Some(n) => n,
                None => continue,
            };

            let kind = rules.classify(&node);

            if kind == target_kind {
                // For definitions, try to find the start of the definition node
                // (e.g., function_definition starts at the return type, not the function name)
                let (def_line, def_col) = if target_kind == SymbolKind::Definition {
                    if let Some(def_node) = find_definition_node(&node, &rules) {
                        let start = def_node.start_position();
                        (start.row + 1, start.column + 1)
                    } else {
                        (m.line as usize, m.column as usize)
                    }
                } else {
                    (m.line as usize, m.column as usize)
                };

                result.push(SemanticMatch {
                    path: path.to_string_lossy().to_string(),
                    line: def_line,
                    column: def_col,
                    text: m.text.clone(),
                    kind,
                });
            }
        }

        result
    }

    fn get_or_parse(&mut self, path: &Path) -> Option<&ParsedFile> {
        // Check local cache first
        if self.local_cache.contains_key(path) {
            return self.local_cache.get(path);
        }

        // Check shared cache
        if let Some(ref shared) = self.shared_cache {
            if let Some((tree, source)) = shared.get(path) {
                let mtime = std::fs::metadata(path).ok()?.modified().ok()?;
                let entry = ParsedFile { tree, source, mtime };
                self.local_cache.insert(path.to_path_buf(), entry);
                return self.local_cache.get(path);
            }
        }

        // Parse from disk
        let source = std::fs::read_to_string(path).ok()?;
        let mtime = std::fs::metadata(path).ok()?.modified().ok()?;
        let tree = self.parser.parse(&source, None)?;

        // Insert into shared cache if available
        if let Some(ref shared) = self.shared_cache {
            shared.insert(path.to_path_buf(), tree.clone(), source.clone());
        }

        let entry = ParsedFile {
            tree,
            source,
            mtime,
        };
        self.local_cache.insert(path.to_path_buf(), entry);
        self.local_cache.get(path)
    }

    pub fn classify_all(&mut self, matches: &[TextMatch]) -> Vec<SemanticMatch> {
        let mut result = Vec::new();
        let rules = self.rules; // Copy rules out

        for m in matches {
            let path = &m.path;
            let parsed = match self.get_or_parse(path) {
                Some(p) => p,
                None => continue,
            };

            let row = (m.line.saturating_sub(1)) as usize;
            let col = (m.column.saturating_sub(1)) as usize;

            let node = match parsed.tree.root_node().descendant_for_point_range(
                tree_sitter::Point::new(row, col),
                tree_sitter::Point::new(row, col + 1),
            ) {
                Some(n) => n,
                None => continue,
            };

            let kind = rules.classify(&node);

            result.push(SemanticMatch {
                path: path.to_string_lossy().to_string(),
                line: m.line as usize,
                column: m.column as usize,
                text: m.text.clone(),
                kind,
            });
        }

        result
    }
}

/// Walk up the AST to find the enclosing definition node.
fn find_definition_node<'a>(node: &'a tree_sitter::Node<'a>, rules: &LanguageRules) -> Option<tree_sitter::Node<'a>> {
    let mut current = Some(*node);
    while let Some(n) = current {
        if rules.definition_ancestors.iter().any(|d| *d == n.kind()) {
            return Some(n);
        }
        current = n.parent();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;

    fn create_cpp_file(dir: &tempfile::TempDir, name: &str, content: &str) -> PathBuf {
        let path = dir.path().join(name);
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(content.as_bytes()).unwrap();
        path
    }

    #[test]
    fn test_filter_definitions_cpp() {
        let dir = tempfile::tempdir().unwrap();
        create_cpp_file(
            &dir,
            "test.cpp",
            r#"void
process_data(int x)
{
}

int main() {
    process_data(42);
    return 0;
}
"#,
        );

        let matches = crate::searcher::search_symbol("process_data", dir.path(), &["cpp"]).unwrap();
        assert_eq!(matches.len(), 2);

        let mut filter = AstFilter::new(LanguageId::Cpp).unwrap();
        let defs = filter.filter_definitions(&matches);
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].line, 1); // corrected to function_definition start
        assert_eq!(defs[0].column, 1);
        assert_eq!(defs[0].kind, SymbolKind::Definition);
    }

    #[test]
    fn test_filter_calls_cpp() {
        let dir = tempfile::tempdir().unwrap();
        create_cpp_file(
            &dir,
            "test.cpp",
            r#"void process_data(int x) {}
int main() {
    process_data(42);
    return 0;
}
"#,
        );

        let matches = crate::searcher::search_symbol("process_data", dir.path(), &["cpp"]).unwrap();
        let mut filter = AstFilter::new(LanguageId::Cpp).unwrap();
        let calls = filter.filter_calls(&matches);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].line, 3);
        assert_eq!(calls[0].kind, SymbolKind::Call);
    }

    #[test]
    fn test_filter_definitions_python() {
        let dir = tempfile::tempdir().unwrap();
        create_cpp_file(
            &dir,
            "test.py",
            r#"def process_data(x):
    print(x)

if __name__ == "__main__":
    process_data(42)
"#,
        );

        let matches = crate::searcher::search_symbol("process_data", dir.path(), &["py"]).unwrap();
        assert_eq!(matches.len(), 2);

        let mut filter = AstFilter::new(LanguageId::Python).unwrap();
        let defs = filter.filter_definitions(&matches);
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].line, 1); // corrected to function_definition start
        assert_eq!(defs[0].kind, SymbolKind::Definition);
    }

    #[test]
    fn test_classify_all() {
        let dir = tempfile::tempdir().unwrap();
        create_cpp_file(
            &dir,
            "test.cpp",
            r#"void process_data(int x) {}
int main() {
    process_data(42);
    return 0;
}
"#,
        );

        let matches = crate::searcher::search_symbol("process_data", dir.path(), &["cpp"]).unwrap();
        let mut filter = AstFilter::new(LanguageId::Cpp).unwrap();
        let classified = filter.classify_all(&matches);
        assert_eq!(classified.len(), 2);

        let def = classified.iter().find(|c| c.line == 1).unwrap();
        assert_eq!(def.kind, SymbolKind::Definition);

        let call = classified.iter().find(|c| c.line == 3).unwrap();
        assert_eq!(call.kind, SymbolKind::Call);
    }

    #[test]
    fn test_filter_discards_comments() {
        let dir = tempfile::tempdir().unwrap();
        create_cpp_file(
            &dir,
            "test.cpp",
            r#"// process_data is a function
void process_data(int x) {}
"#,
        );

        let matches = crate::searcher::search_symbol("process_data", dir.path(), &["cpp"]).unwrap();
        let mut filter = AstFilter::new(LanguageId::Cpp).unwrap();
        let defs = filter.filter_definitions(&matches);
        // The comment line should not appear as a definition
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].line, 2);
    }

    #[test]
    fn test_shared_cache() {
        let dir = tempfile::tempdir().unwrap();
        create_cpp_file(&dir, "test.cpp", "void process_data(int x) {}\n");

        let cache = Arc::new(AstCache::new(10));
        let matches = crate::searcher::search_symbol("process_data", dir.path(), &["cpp"]).unwrap();

        let mut filter1 = AstFilter::new_with_cache(LanguageId::Cpp, Some(Arc::clone(&cache))).unwrap();
        let defs1 = filter1.filter_definitions(&matches);
        assert_eq!(defs1.len(), 1);

        // Second filter with the same shared cache should reuse the parsed tree
        let mut filter2 = AstFilter::new_with_cache(LanguageId::Cpp, Some(Arc::clone(&cache))).unwrap();
        let defs2 = filter2.filter_definitions(&matches);
        assert_eq!(defs2.len(), 1);
        assert_eq!(defs1[0].line, defs2[0].line);
    }
}
