use std::collections::HashMap;
use tree_sitter::Language;

/// Supported programming languages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum LanguageId {
    C,
    Cpp,
    Python,
}

impl LanguageId {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "c" => Some(Self::C),
            "cpp" | "c++" | "cxx" | "cc" => Some(Self::Cpp),
            "python" | "py" => Some(Self::Python),
            _ => None,
        }
    }

    pub fn to_tree_sitter_language(self) -> Language {
        match self {
            LanguageId::C => tree_sitter_c::LANGUAGE.into(),
            LanguageId::Cpp => tree_sitter_cpp::LANGUAGE.into(),
            LanguageId::Python => tree_sitter_python::LANGUAGE.into(),
        }
    }

    /// File extensions associated with this language.
    pub fn extensions(self) -> &'static [&'static str] {
        match self {
            LanguageId::C => &["c", "h"],
            LanguageId::Cpp => &["cpp", "cc", "cxx", "hpp", "hh", "hxx"],
            LanguageId::Python => &["py"],
        }
    }
}

/// Describes what a matched AST node represents semantically.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SymbolKind {
    Definition,
    Call,
    Reference,
    Comment,
    String,
    Other,
}

/// Mapping from tree-sitter node type names to semantic kinds for definition detection.
#[derive(Clone, Copy)]
pub struct LanguageRules {
    /// Node types that indicate a definition when matched.
    pub definition_ancestors: &'static [&'static str],
    /// Node types that indicate a call expression.
    pub call_ancestors: &'static [&'static str],
    /// Node types to discard (comments, strings, etc.).
    pub discard_ancestors: &'static [&'static str],
}

impl LanguageRules {
    pub fn for_language(lang: LanguageId) -> Self {
        match lang {
            LanguageId::C | LanguageId::Cpp => Self {
                definition_ancestors: &[
                    "function_definition",
                    "class_definition",
                    "struct_specifier",
                    "enum_specifier",
                    "union_specifier",
                    "declaration",
                    "field_declaration",
                    "parameter_declaration",
                    "template_declaration",
                    "namespace_definition",
                ],
                call_ancestors: &["call_expression", "field_expression"],
                discard_ancestors: &[
                    "comment",
                    "string_literal",
                    "raw_string_literal",
                    "system_lib_string",
                ],
            },
            LanguageId::Python => Self {
                definition_ancestors: &[
                    "function_definition",
                    "class_definition",
                    " decorated_definition",
                    "parameter",
                ],
                call_ancestors: &["call"],
                discard_ancestors: &["comment", "string", "concatenated_string"],
            },
        }
    }

    pub fn classify(&self, node: &tree_sitter::Node) -> SymbolKind {
        let mut current = Some(*node);
        while let Some(n) = current {
            let kind = n.kind();
            if self.discard_ancestors.iter().any(|d| *d == kind) {
                return SymbolKind::Comment;
            }
            // Check call before definition, because a call inside a
            // function_definition should be classified as Call, not Definition.
            if self.call_ancestors.iter().any(|d| *d == kind) {
                return SymbolKind::Call;
            }
            if self.definition_ancestors.iter().any(|d| *d == kind) {
                return SymbolKind::Definition;
            }
            current = n.parent();
        }
        SymbolKind::Reference
    }
}

/// Attempt to guess language from file path.
pub fn guess_language_from_path(path: &std::path::Path) -> Option<LanguageId> {
    let ext = path.extension()?.to_str()?;
    let map: HashMap<&str, LanguageId> = [
        ("c", LanguageId::C),
        ("h", LanguageId::C),
        ("cpp", LanguageId::Cpp),
        ("cc", LanguageId::Cpp),
        ("cxx", LanguageId::Cpp),
        ("hpp", LanguageId::Cpp),
        ("hh", LanguageId::Cpp),
        ("hxx", LanguageId::Cpp),
        ("py", LanguageId::Python),
    ]
    .into_iter()
    .collect();
    map.get(ext).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_language_id_from_str() {
        assert_eq!(LanguageId::from_str("c"), Some(LanguageId::C));
        assert_eq!(LanguageId::from_str("C"), Some(LanguageId::C));
        assert_eq!(LanguageId::from_str("cpp"), Some(LanguageId::Cpp));
        assert_eq!(LanguageId::from_str("c++"), Some(LanguageId::Cpp));
        assert_eq!(LanguageId::from_str("cxx"), Some(LanguageId::Cpp));
        assert_eq!(LanguageId::from_str("cc"), Some(LanguageId::Cpp));
        assert_eq!(LanguageId::from_str("python"), Some(LanguageId::Python));
        assert_eq!(LanguageId::from_str("py"), Some(LanguageId::Python));
        assert_eq!(LanguageId::from_str("rust"), None);
    }

    #[test]
    fn test_guess_language_from_path() {
        assert_eq!(
            guess_language_from_path(std::path::Path::new("foo.cpp")),
            Some(LanguageId::Cpp)
        );
        assert_eq!(
            guess_language_from_path(std::path::Path::new("bar.c")),
            Some(LanguageId::C)
        );
        assert_eq!(
            guess_language_from_path(std::path::Path::new("baz.py")),
            Some(LanguageId::Python)
        );
        assert_eq!(
            guess_language_from_path(std::path::Path::new("README.md")),
            None
        );
    }

    #[test]
    fn test_cpp_extensions() {
        assert_eq!(LanguageId::C.extensions(), &["c", "h"]);
        assert_eq!(
            LanguageId::Cpp.extensions(),
            &["cpp", "cc", "cxx", "hpp", "hh", "hxx"]
        );
        assert_eq!(LanguageId::Python.extensions(), &["py"]);
    }
}
