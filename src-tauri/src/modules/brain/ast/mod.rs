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

/// TS/TSX scope anchor (ADR-010 cluster 7): a capture only counts as a DEFINITION
/// when every enclosing scope is module- or class-like. The patterns above are
/// unanchored, so without this a function-local `const`, a nested helper, an
/// object-literal method, or a loop-header variable would register as a project
/// symbol — graph noise that poisons impact analysis and the Brain Map. Walking up
/// from the captured name: any function body (a `statement_block` not owned by a
/// namespace/module), object literal, function expression, or loop header means
/// the name is runtime-local. Rust needs no anchor (`let` locals aren't captured;
/// nested items are real, named definitions).
fn is_ts_module_scoped(name_node: tree_sitter::Node) -> bool {
    let mut cur = name_node.parent();
    while let Some(n) = cur {
        match n.kind() {
            // Namespace/module bodies are statement_blocks too — those stay allowed,
            // as is `declare global { … }`, whose statement_block is owned directly
            // by ambient_declaration (module/global scope, not a function body).
            "statement_block"
                if !matches!(
                    n.parent().map(|p| p.kind()),
                    Some("internal_module" | "module" | "ambient_declaration")
                ) =>
            {
                return false;
            }
            "object" | "arrow_function" | "function_expression" | "generator_function"
            | "for_statement" | "for_in_statement" => return false,
            _ => {}
        }
        cur = n.parent();
    }
    true
}

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
            if matches!(lang, Lang::TypeScript | Lang::Tsx) && !is_ts_module_scoped(cap.node) {
                continue; // function-local / object-literal — not a definition
            }
            if let Ok(t) = cap.node.utf8_text(src) {
                if !t.is_empty() {
                    out.push(t.to_string());
                }
            }
        }
    }
    out
}

/// A definition node (for the graph + `brain_get_symbol`).
#[derive(Clone, Debug)]
pub struct CodeNode {
    pub name: String,
    pub kind: String,
    pub start_line: i64,
    /// 0-based column — disambiguates same-line same-kind defs (e.g. a getter and
    /// setter of the same property on one line) so neither is dropped.
    pub start_col: i64,
}

/// One file's AST analysis: definitions + raw import specifiers (one parse).
#[derive(Clone, Debug, Default)]
pub struct Analysis {
    pub nodes: Vec<CodeNode>,
    pub imports: Vec<String>,
}

impl Analysis {
    /// Space-joined definition names for the FTS `symbols` column.
    pub fn symbol_names(&self) -> String {
        self.nodes
            .iter()
            .map(|n| n.name.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// A definition location (`brain_get_symbol` result).
#[derive(Clone, Debug, serde::Serialize)]
pub struct SymbolInfo {
    pub path: String,
    pub name: String,
    pub kind: String,
    pub start_line: i64,
}

/// Tiered impact (`brain_code_impact`): AST-confident reverse-import dependents
/// vs the lexical over-approximation (CONCEPT §4.1b).
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct Impact {
    pub symbol: String,
    pub defined_in: Vec<String>,
    pub ast_dependents: Vec<String>,
    pub lexical_candidates: Vec<String>,
}

fn normalize_kind(raw: &str) -> &'static str {
    match raw {
        "function_item" | "function_declaration" | "function_signature_item" => "function",
        "struct_item" => "struct",
        "enum_item" | "enum_declaration" => "enum",
        "union_item" => "union",
        "trait_item" => "trait",
        "type_item" | "type_alias_declaration" => "type",
        "const_item" | "static_item" | "variable_declarator" => "const",
        "mod_item" => "module",
        "macro_definition" => "macro",
        "class_declaration" | "abstract_class_declaration" => "class",
        "method_definition" => "method",
        "interface_declaration" => "interface",
        "public_field_definition" => "field",
        _ => "symbol",
    }
}

// Static + re-export import specifiers (TS/JS/TSX). Rust `use` paths aren't file
// paths and need module-tree resolution — deferred (Rust still gets nodes).
const TS_IMPORTS: &str = r#"
(import_statement source: (string) @src)
(export_statement source: (string) @src)
"#;

fn imports_query(lang: Lang) -> Option<&'static str> {
    match lang {
        Lang::TypeScript | Lang::Tsx => Some(TS_IMPORTS),
        Lang::Rust => None,
    }
}

/// Parse once and extract both definitions and import specifiers. Fail-open.
pub fn analyze(lang: Lang, source: &str) -> Analysis {
    let Some(tree) = parse(lang, source) else {
        return Analysis::default();
    };
    let language = language(lang);
    let src = source.as_bytes();
    Analysis {
        nodes: run_defs(&language, lang, &tree, src),
        imports: run_imports(&language, lang, &tree, src),
    }
}

fn run_defs(language: &Language, lang: Lang, tree: &Tree, src: &[u8]) -> Vec<CodeNode> {
    let query = match Query::new(language, defs_query(lang)) {
        Ok(q) => q,
        Err(e) => {
            log::warn!("brain: defs query failed for {lang:?}: {e}");
            return Vec::new();
        }
    };
    let mut cursor = QueryCursor::new();
    let mut out = Vec::new();
    let mut it = cursor.matches(&query, tree.root_node(), src);
    while let Some(m) = it.next() {
        for cap in m.captures {
            let node = cap.node;
            if matches!(lang, Lang::TypeScript | Lang::Tsx) && !is_ts_module_scoped(node) {
                continue; // function-local / object-literal — not a definition
            }
            if let Ok(name) = node.utf8_text(src) {
                if name.is_empty() {
                    continue;
                }
                let kind = node.parent().map(|p| normalize_kind(p.kind())).unwrap_or("symbol");
                let pos = node.start_position();
                out.push(CodeNode {
                    name: name.to_string(),
                    kind: kind.to_string(),
                    start_line: pos.row as i64 + 1,
                    start_col: pos.column as i64,
                });
            }
        }
    }
    out
}

fn run_imports(language: &Language, lang: Lang, tree: &Tree, src: &[u8]) -> Vec<String> {
    let Some(q_src) = imports_query(lang) else {
        return Vec::new();
    };
    let query = match Query::new(language, q_src) {
        Ok(q) => q,
        Err(e) => {
            log::warn!("brain: imports query failed for {lang:?}: {e}");
            return Vec::new();
        }
    };
    let mut cursor = QueryCursor::new();
    let mut out = Vec::new();
    let mut it = cursor.matches(&query, tree.root_node(), src);
    while let Some(m) = it.next() {
        for cap in m.captures {
            if let Ok(raw) = cap.node.utf8_text(src) {
                let spec = raw.trim_matches(['"', '\'', '`']);
                if !spec.is_empty() {
                    out.push(spec.to_string());
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

    /// ADR-010 cluster 7: TS definition queries are scope-anchored. Function-local
    /// variables, nested helpers, object-literal methods, and loop-header variables
    /// must NOT register as definitions; top-level/module/class-member ones must.
    #[test]
    fn ts_defs_are_scope_anchored() {
        let src = r#"
export function outer() {
  const localVar = 1;
  function innerHelper() {}
  const cb = () => { const arrowLocal = 2; return arrowLocal; };
  for (const loopVar of [1]) {}
  return { objMethod() { return localVar + cb(); } };
}
const topConst = 3;
class Klass {
  method() { const methodLocal = 4; return methodLocal; }
  field = 5;
}
namespace Ns { export const nsConst = 6; }
"#;
        let a = analyze(Lang::TypeScript, src);
        let names: Vec<&str> = a.nodes.iter().map(|n| n.name.as_str()).collect();
        for keep in ["outer", "topConst", "Klass", "method", "field", "nsConst"] {
            assert!(names.contains(&keep), "missing real def {keep} in {names:?}");
        }
        for noise in ["localVar", "innerHelper", "arrowLocal", "loopVar", "objMethod", "methodLocal"] {
            assert!(!names.contains(&noise), "local {noise} must not be a node: {names:?}");
        }
        // The FTS `symbols` column path (extract_defs) applies the same anchor.
        let d = extract_defs(Lang::TypeScript, src);
        assert!(d.contains(&"topConst".to_string()));
        assert!(!d.contains(&"localVar".to_string()), "symbols column must skip locals: {d:?}");
    }

    /// ADR-010 cluster 7 repair: `declare global { … }` bodies are statement_blocks
    /// owned by `ambient_declaration` — genuinely global-scoped declarations (the
    /// common Window/globalThis augmentation pattern) that the scope anchor must
    /// keep, alongside `declare module "x" { … }` (parent kind `module`).
    #[test]
    fn ts_declare_global_defs_are_kept() {
        let src = r#"
declare global {
  interface KodenBridge { invoke(cmd: string): Promise<unknown> }
  var kodenFlag: boolean;
}
declare module "some-mod" { export const modConst: number; }
"#;
        let d = extract_defs(Lang::TypeScript, src);
        for keep in ["KodenBridge", "kodenFlag", "modConst"] {
            assert!(d.contains(&keep.to_string()), "missing global/ambient def {keep} in {d:?}");
        }
    }

    #[test]
    fn analyze_extracts_nodes_and_imports() {
        let a = analyze(
            Lang::TypeScript,
            "import { x } from './a';\nexport { y } from '../b';\nexport function foo() {}",
        );
        assert!(a.nodes.iter().any(|n| n.name == "foo" && n.kind == "function"));
        assert!(a.imports.contains(&"./a".to_string()));
        assert!(a.imports.contains(&"../b".to_string()));
        // Rust: nodes extracted, but import edges deferred (use-path resolution).
        let r = analyze(Lang::Rust, "use crate::foo;\npub fn bar() {}");
        assert!(r.nodes.iter().any(|n| n.name == "bar"));
        assert!(r.imports.is_empty());
    }
}
