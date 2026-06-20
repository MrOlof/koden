//! tree-sitter AST layer (ADR-006 P2) — the marquee upgrade over Conductr's
//! regex symbol extraction. v1 grammars: Rust + TypeScript (+TSX, which also
//! covers JS/JSX since TS is a superset for our def-extraction purposes), pinned
//! via the resolved crate versions. CI smoke-parses one fixture per language so
//! ABI drift (the ADR's top risk) fails loudly.
//!
//! This increment extracts top-level DEFINITION names to populate the FTS
//! `symbols` column (left empty in P0), so identifier search ranks real defs.
//! The full graph (imports/refs/calls + forward/reverse adjacency + impact +
//! incremental relink) builds on this in later P2 increments.

use tree_sitter::{Language, Parser, Query, QueryCursor, StreamingIterator, Tree};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    Rust,
    TypeScript,
    Tsx,
}

/// Map a file extension to a grammar. JS/JSX ride the TS/TSX grammars (superset).
pub fn lang_for_ext(ext: &str) -> Option<Lang> {
    match ext.to_ascii_lowercase().as_str() {
        "rs" => Some(Lang::Rust),
        "ts" | "mts" | "cts" | "js" | "mjs" | "cjs" => Some(Lang::TypeScript),
        "tsx" | "jsx" => Some(Lang::Tsx),
        _ => None,
    }
}

fn language(lang: Lang) -> Language {
    match lang {
        Lang::Rust => tree_sitter_rust::LANGUAGE.into(),
        Lang::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        Lang::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
    }
}

/// Parse source into a tree. `None` if the grammar can't load (ABI mismatch) or
/// parsing returns nothing — callers degrade to lexical-only (fail-open).
pub fn parse(lang: Lang, source: &str) -> Option<Tree> {
    let mut parser = Parser::new();
    parser.set_language(&language(lang)).ok()?;
    parser.parse(source, None)
}

// Definition-name capture queries. Methods inside Rust `impl` blocks are
// `function_item`, already covered. Best-effort: an invalid node kind makes
// Query::new fail and extraction returns [] (logged), never panics.
const RUST_DEFS: &str = r#"
(function_item name: (identifier) @name)
(struct_item name: (type_identifier) @name)
(enum_item name: (type_identifier) @name)
(union_item name: (type_identifier) @name)
(trait_item name: (type_identifier) @name)
(type_item name: (type_identifier) @name)
(const_item name: (identifier) @name)
(static_item name: (identifier) @name)
(mod_item name: (identifier) @name)
(macro_definition name: (identifier) @name)
"#;

const TS_DEFS: &str = r#"
(function_declaration name: (identifier) @name)
(class_declaration name: (type_identifier) @name)
(abstract_class_declaration name: (type_identifier) @name)
(method_definition name: (property_identifier) @name)
(interface_declaration name: (type_identifier) @name)
(type_alias_declaration name: (type_identifier) @name)
(enum_declaration name: (identifier) @name)
(variable_declarator name: (identifier) @name)
(public_field_definition name: (property_identifier) @name)
"#;

fn defs_query(lang: Lang) -> &'static str {
    match lang {
        Lang::Rust => RUST_DEFS,
        Lang::TypeScript | Lang::Tsx => TS_DEFS,
    }
}

/// Extract definition names for the `symbols` FTS column. Fail-open: `[]` on any
/// grammar/query error.
pub fn extract_defs(lang: Lang, source: &str) -> Vec<String> {
    let Some(tree) = parse(lang, source) else {
        return Vec::new();
    };
    let language = language(lang);
    let query = match Query::new(&language, defs_query(lang)) {
        Ok(q) => q,
        Err(e) => {
            log::warn!("brain: defs query failed for {lang:?}: {e}");
            return Vec::new();
        }
    };
    let src = source.as_bytes();
    let mut cursor = QueryCursor::new();
    let mut out = Vec::new();
    let mut it = cursor.matches(&query, tree.root_node(), src);
    while let Some(m) = it.next() {
        for cap in m.captures {
            if let Ok(t) = cap.node.utf8_text(src) {
                if !t.is_empty() {
                    out.push(t.to_string());
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ext_mapping() {
        assert_eq!(lang_for_ext("rs"), Some(Lang::Rust));
        assert_eq!(lang_for_ext("ts"), Some(Lang::TypeScript));
        assert_eq!(lang_for_ext("JSX"), Some(Lang::Tsx));
        assert_eq!(lang_for_ext("py"), None);
    }

    #[test]
    fn grammars_load_and_parse() {
        // Proves the grammar ABI matches the core (set_language + parse succeed).
        let rt = parse(Lang::Rust, "pub fn foo() -> i32 { 1 }").expect("rust parse");
        assert_eq!(rt.root_node().kind(), "source_file");
        assert!(!rt.root_node().has_error());
        let ts = parse(Lang::TypeScript, "export function bar(x: number) { return x; }")
            .expect("ts parse");
        assert_eq!(ts.root_node().kind(), "program");
        let tsx = parse(Lang::Tsx, "const A = () => <div/>;").expect("tsx parse");
        assert!(!tsx.root_node().has_error());
    }

    #[test]
    fn extracts_rust_defs() {
        let d = extract_defs(
            Lang::Rust,
            "pub fn alpha() {}\nstruct Bravo;\nenum Charlie {}\ntrait Delta {}\nconst ECHO: i32 = 1;\nmod foxtrot {}",
        );
        for n in ["alpha", "Bravo", "Charlie", "Delta", "ECHO", "foxtrot"] {
            assert!(d.contains(&n.to_string()), "missing {n} in {d:?}");
        }
    }

    #[test]
    fn extracts_ts_defs() {
        let d = extract_defs(
            Lang::TypeScript,
            "export function alpha(){}\nclass Bravo{}\ninterface Charlie{}\ntype Delta = number;\nconst echo = 1;\nenum Foxtrot{}",
        );
        for n in ["alpha", "Bravo", "Charlie", "Delta", "echo", "Foxtrot"] {
            assert!(d.contains(&n.to_string()), "missing {n} in {d:?}");
        }
    }
}
