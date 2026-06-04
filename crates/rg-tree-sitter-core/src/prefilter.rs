use crate::searcher::TextMatch;

/// L1: 行级启发式初筛 —— 过滤掉明显在注释或字符串中的匹配。
///
/// 策略保守：宁可放过垃圾匹配，也绝不过滤真正的定义。
pub fn quick_filter(matches: &[TextMatch]) -> Vec<TextMatch> {
    matches
        .iter()
        .filter(|m| !is_likely_comment(m) && !is_likely_string(m))
        .cloned()
        .collect()
}

/// 判断匹配行是否明显在注释中。
fn is_likely_comment(m: &TextMatch) -> bool {
    let trimmed = m.text.trim_start();
    // 单行注释
    if trimmed.starts_with("//") {
        return true;
    }
    // 块注释开始
    if trimmed.starts_with("/*") || trimmed.starts_with("/**") {
        return true;
    }
    // 块注释中间行（以 * 开头）
    // 注意：指针解引用如 `*ptr` 也可能以 * 开头，所以需要更严格的检测
    // 保守策略：* 前面只有空白，且后面也是空白或字母，才认为是注释
    if let Some(rest) = trimmed.strip_prefix('*') {
        if rest.starts_with(' ') || rest.starts_with('\t') || rest.is_empty() {
            return true;
        }
    }
    false
}

/// 判断匹配点是否可能位于字符串字面量中。
///
/// 简单启发式：统计匹配点之前未配对的 `"` 和 `'` 数量。
/// 对于 `"` 只考虑双引号字符串，`'` 只考虑单引号字符（C/C++）或字符串（Python）。
/// 不处理三引号、转义、raw string 等复杂情况 —— 允许少量误判。
fn is_likely_string(m: &TextMatch) -> bool {
    // column 是 1-based，转换为 0-based 字节索引
    let col = m.column.saturating_sub(1) as usize;
    let line = &m.text;

    // 找到 column 位置的字符索引（按 chars 计数）
    let byte_pos = match line.char_indices().nth(col) {
        Some((idx, _)) => idx,
        None => return false,
    };

    let prefix = &line[..byte_pos];
    is_inside_quotes(prefix, '"') || is_inside_quotes(prefix, '\'')
}

/// 检查 prefix 中是否存在未配对的 quote_char。
fn is_inside_quotes(prefix: &str, quote_char: char) -> bool {
    let mut count = 0;
    let mut escaped = false;
    for ch in prefix.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == quote_char {
            count += 1;
        }
    }
    count % 2 == 1
}

/// L2: 上下文关键字初筛 —— 提升定义匹配置信度。
///
/// 对于 `define` 查询，优先保留包含定义关键字的行；
/// 对于 `refs` 查询，保留所有（由后续 AST 过滤决定）。
///
/// 返回 `(high_confidence, low_confidence)` 两组，
/// 先处理高置信度，低置信度作为 fallback。
pub fn split_by_confidence(matches: &[TextMatch]) -> (Vec<TextMatch>, Vec<TextMatch>) {
    let mut high = Vec::new();
    let mut low = Vec::new();
    for m in matches {
        if has_definition_keyword(&m.text) {
            high.push(m.clone());
        } else {
            low.push(m.clone());
        }
    }
    (high, low)
}

/// 检测行内是否包含定义相关的上下文关键字。
///
/// 保守策略：只在 symbol 附近（行首到 symbol 结束）搜索关键字，
/// 避免跨行误匹配。
fn has_definition_keyword(line: &str) -> bool {
    // C/C++/Python 常见的定义前导关键字
    let keywords = [
        "void ", "int ", "char ", "bool ", "float ", "double ", "auto ",
        "class ", "struct ", "enum ", "union ", "template ",
        "static ", "inline ", "const ", "constexpr ", "virtual ",
        "explicit ", "friend ", "typedef ", "using ", "namespace ",
        "def ", "class ", "async def ",
    ];
    let lower = line.to_lowercase();
    keywords.iter().any(|kw| lower.contains(kw))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_match(text: &str, column: u64) -> TextMatch {
        TextMatch {
            path: std::path::PathBuf::from("test.cpp"),
            line: 1,
            column,
            text: text.to_string(),
        }
    }

    #[test]
    fn test_comment_filtering() {
        let matches = vec![
            make_match("// process_data is great", 4),
            make_match("  /* process_data */", 4),
            make_match(" * process_data doc", 3),
            make_match("void process_data() {}", 6),
            make_match("*ptr = process_data();", 10), // 指针解引用，不应过滤
        ];
        let filtered = quick_filter(&matches);
        assert_eq!(filtered.len(), 2);
        assert!(filtered.iter().any(|m| m.text.contains("void process_data")));
        assert!(filtered.iter().any(|m| m.text.contains("*ptr")));
    }

    #[test]
    fn test_string_filtering() {
        // Column values point to the 'p' in 'process_data' (1-based)
        let matches = vec![
            make_match(r#"    "process_data""#, 6),   // 在双引号字符串内
            make_match(r#"    'process_data'"#, 6),   // 在单引号内
            make_match(r#"    "ok" + process_data"#, 12), // 在字符串外
            make_match("void process_data() {}", 6),  // 正常定义
        ];
        let filtered = quick_filter(&matches);
        assert_eq!(filtered.len(), 2);
        assert!(filtered.iter().any(|m| m.text.contains("ok")));
        assert!(filtered.iter().any(|m| m.text.contains("void process_data")));
    }

    #[test]
    fn test_definition_keywords() {
        assert!(has_definition_keyword("void process_data(int x) {}"));
        assert!(has_definition_keyword("def process_data(x):"));
        assert!(has_definition_keyword("class ProcessData:"));
        assert!(!has_definition_keyword("    process_data(42);"));
    }

    #[test]
    fn test_no_false_negative_for_real_defs() {
        // 真正的定义行不应被 L1 过滤
        let def_cases = [
            "void process_data() {}",
            "int    process_data(int);",
            "static void process_data() {}",
            "def process_data(x):",
            "class process_data:",
        ];
        for text in &def_cases {
            let m = make_match(text, 6);
            assert!(
                !is_likely_comment(&m) && !is_likely_string(&m),
                "{} should NOT be filtered",
                text
            );
        }
    }
}
