use grep_searcher::{Searcher, SearcherBuilder, Sink, SinkMatch};
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// A single text match found by the searcher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextMatch {
    pub path: PathBuf,
    pub line: u64,
    pub column: u64,
    pub text: String,
}

struct MatchCollector {
    path: PathBuf,
    symbol: String,
    matches: Arc<Mutex<Vec<TextMatch>>>,
}

impl Sink for MatchCollector {
    type Error = io::Error;

    fn matched(
        &mut self,
        _searcher: &Searcher,
        mat: &SinkMatch<'_>,
    ) -> Result<bool, io::Error> {
        let line_number = mat.line_number().unwrap_or(1);
        let matched_bytes = mat.bytes();

        let text = match std::str::from_utf8(matched_bytes) {
            Ok(s) => s.trim_end(),
            Err(_) => return Ok(true), // skip invalid UTF-8
        };

        // The symbol we're searching for is embedded somewhere in this line.
        // Find its first occurrence to compute the column.
        let symbol = self.symbol.as_str();
        let col_offset = match text.find(symbol) {
            Some(off) => off,
            None => return Ok(true),
        };
        let column = text[..col_offset].chars().count() as u64 + 1;

        self.matches.lock().unwrap().push(TextMatch {
            path: self.path.clone(),
            line: line_number,
            column,
            text: text.to_string(),
        });
        Ok(true)
    }
}

/// Search for `symbol` in files under `dir`, filtered by file extensions.
pub fn search_symbol(
    symbol: &str,
    dir: &std::path::Path,
    extensions: &[&str],
) -> anyhow::Result<Vec<TextMatch>> {
    let matcher = grep_regex::RegexMatcherBuilder::new()
        .build(&regex::escape(symbol))?;

    let mut searcher = SearcherBuilder::new()
        .line_number(true)
        .build();

    let matches: Arc<Mutex<Vec<TextMatch>>> = Arc::new(Mutex::new(Vec::new()));

    let types = if extensions.is_empty() {
        ignore::types::TypesBuilder::new().build()?
    } else {
        let mut builder = ignore::types::TypesBuilder::new();
        for ext in extensions {
            builder.add("custom", &format!("*.{ext}"))?;
        }
        builder.select("custom");
        builder.build()?
    };

    let walker = ignore::WalkBuilder::new(dir)
        .types(types)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .build();

    for result in walker {
        let entry = match result {
            Ok(e) => e,
            Err(_) => continue,
        };

        if !entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
            continue;
        }

        let path = entry.path().to_path_buf();
        let sink = MatchCollector {
            path: path.clone(),
            symbol: symbol.to_string(),
            matches: Arc::clone(&matches),
        };

        let _ = searcher.search_path(&matcher, &path, sink);
    }

    let result = Arc::try_unwrap(matches)
        .expect("Arc still has multiple owners")
        .into_inner()
        .unwrap();
    Ok(result)
}

/// Parse matches from ripgrep-style plain text input.
/// Expected format: `path:line:column:text`
pub fn parse_external_matches<R: std::io::Read>(input: R) -> anyhow::Result<Vec<TextMatch>> {
    use std::io::{BufRead, BufReader};
    let reader = BufReader::new(input);
    let mut matches = Vec::new();

    for line in reader.lines() {
        let line = line?;
        // Format: file:line:col:text
        let mut parts = line.splitn(4, ':');
        let path = match parts.next() {
            Some(p) => PathBuf::from(p),
            None => continue,
        };
        let line_num: u64 = match parts.next().and_then(|s| s.parse().ok()) {
            Some(n) => n,
            None => continue,
        };
        let col: u64 = match parts.next().and_then(|s| s.parse().ok()) {
            Some(n) => n,
            None => continue,
        };
        let text = parts.next().unwrap_or("").to_string();
        matches.push(TextMatch {
            path,
            line: line_num,
            column: col,
            text,
        });
    }

    Ok(matches)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_parse_external_matches_basic() {
        let input = "src/main.cpp:10:5:void foo():\nsrc/lib.rs:20:1:fn bar(){}";
        let matches = parse_external_matches(Cursor::new(input)).unwrap();
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].path, PathBuf::from("src/main.cpp"));
        assert_eq!(matches[0].line, 10);
        assert_eq!(matches[0].column, 5);
        assert_eq!(matches[0].text, "void foo():");
        assert_eq!(matches[1].path, PathBuf::from("src/lib.rs"));
        assert_eq!(matches[1].line, 20);
        assert_eq!(matches[1].column, 1);
        assert_eq!(matches[1].text, "fn bar(){}");
    }

    #[test]
    fn test_parse_external_matches_empty_text() {
        let input = "src/a.cpp:1:1:";
        let matches = parse_external_matches(Cursor::new(input)).unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].text, "");
    }

    #[test]
    fn test_search_symbol_basic() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.cpp");
        std::fs::write(
            &file_path,
            r#"void process_data(int x) {}
int main() {
    process_data(42);
    return 0;
}
"#,
        )
        .unwrap();

        let results = search_symbol("process_data", dir.path(), &["cpp"]).unwrap();
        assert_eq!(results.len(), 2);

        // Definition should be on line 1
        let def = results.iter().find(|m| m.line == 1).unwrap();
        assert_eq!(def.column, 6);
        assert!(def.text.contains("process_data"));

        // Call should be on line 3
        let call = results.iter().find(|m| m.line == 3).unwrap();
        assert_eq!(call.column, 5);
        assert!(call.text.contains("process_data"));
    }

    #[test]
    fn test_search_symbol_no_match() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("test.cpp"), "void foo() {}\n").unwrap();
        let results = search_symbol("nonexistent", dir.path(), &["cpp"]).unwrap();
        assert!(results.is_empty());
    }
}
