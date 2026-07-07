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

/// BFS direction for `brain_code_impact` over the file-level import graph.
/// Upstream = who depends on the defining files (reverse-import, dst→src);
/// Downstream = what the defining files depend on (src→dst); Both = merged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImpactDirection {
    Upstream,
    Downstream,
    Both,
}

impl ImpactDirection {
    /// Strict parse — an unknown direction is a caller error, not a silent default.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "upstream" => Some(Self::Upstream),
            "downstream" => Some(Self::Downstream),
            "both" => Some(Self::Both),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Upstream => "upstream",
            Self::Downstream => "downstream",
            Self::Both => "both",
        }
    }
}

/// One depth-annotated AST impact row. `depth` = minimal BFS hops from the
/// NEAREST defining file (1 = a direct import edge). File-granular: our graph
/// has file-level import edges, not symbol-level ones.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct ImpactRow {
    pub path: String,
    pub depth: usize,
}

/// Tiered impact (`brain_code_impact`): AST-confident reverse-import dependents
/// vs the lexical over-approximation (CONCEPT §4.1b). New fields are additive
/// (serde output only) — `ast_dependents` stays as the flat wire-compat list,
/// now mirroring `rows` (same order) instead of being alphabetically resorted.
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct Impact {
    pub symbol: String,
    /// "upstream" | "downstream" | "both" (empty only in the fail-open default).
    pub direction: String,
    pub defined_in: Vec<String>,
    /// Flat mirror of `rows` (same order, same truncation) — wire compatibility.
    pub ast_dependents: Vec<String>,
    /// Depth-annotated AST reach, ordered (depth asc, path asc); truncation is
    /// applied AFTER that full ordering so the kept prefix is stable.
    pub rows: Vec<ImpactRow>,
    /// Lexical over-approximation tier (content mentions), capped at 50, sorted.
    /// NEVER depth-annotated — these are not graph-confirmed edges.
    pub lexical_candidates: Vec<String>,
    /// True when `rows` was cut at `max_results`.
    pub truncated: bool,
    /// Pre-truncation row count (post `exclude_tests` filtering).
    pub result_total: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncated_reason: Option<String>,
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

// Static + re-export import specifiers (TS/JS/TSX).
const TS_IMPORTS: &str = r#"
(import_statement source: (string) @src)
(export_statement source: (string) @src)
"#;

// Rust `use` declarations (incl. `pub use` re-exports). The whole argument is
// captured and expanded textually (groups/aliases/wildcards) into individual
// `::`-paths — module-tree RESOLUTION to file paths happens in the store
// (`rust_use_base`), mirroring the TS split (raw spec here, resolution there).
const RUST_IMPORTS: &str = r#"
(use_declaration argument: (_) @use)
"#;

fn imports_query(lang: Lang) -> Option<&'static str> {
    match lang {
        Lang::TypeScript | Lang::Tsx => Some(TS_IMPORTS),
        Lang::Rust => Some(RUST_IMPORTS),
    }
}

/// Expand one `use` argument into flat `::`-paths: groups recurse
/// (`a::{b, c::d}` → `a::b`, `a::c::d`), `self` in a group and trailing
/// wildcards collapse to their prefix (`a::{self}` / `a::*` → `a`), and
/// ` as alias` is dropped. Malformed text (unbalanced braces) yields nothing —
/// fail-open, like every other extraction path here.
fn expand_rust_use(prefix: &str, part: &str, out: &mut Vec<String>) {
    let part = part.trim();
    if part.is_empty() {
        return;
    }
    if let Some(open) = part.find('{') {
        let Some(close) = part.rfind('}') else {
            return; // unbalanced — skip this use
        };
        if close < open {
            return;
        }
        let head = part[..open].trim().trim_end_matches("::").trim();
        let new_prefix = join_use_path(prefix, head);
        // Split the group body on top-level commas only (nested groups recurse).
        let inner = &part[open + 1..close];
        let mut depth = 0usize;
        let mut start = 0usize;
        for (i, c) in inner.char_indices() {
            match c {
                '{' => depth += 1,
                '}' => depth = depth.saturating_sub(1),
                ',' if depth == 0 => {
                    expand_rust_use(&new_prefix, &inner[start..i], out);
                    start = i + 1;
                }
                _ => {}
            }
        }
        expand_rust_use(&new_prefix, &inner[start..], out);
        return;
    }
    let leaf = part.split(" as ").next().unwrap_or(part).trim();
    let leaf = leaf.strip_suffix("::*").unwrap_or(leaf).trim();
    let leaf = if leaf == "*" || leaf == "self" { "" } else { leaf };
    let full = join_use_path(prefix, leaf);
    if !full.is_empty() {
        out.push(full);
    }
}

fn join_use_path(prefix: &str, seg: &str) -> String {
    match (prefix.is_empty(), seg.is_empty()) {
        (true, _) => seg.to_string(),
        (_, true) => prefix.to_string(),
        _ => format!("{prefix}::{seg}"),
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
                if lang == Lang::Rust {
                    expand_rust_use("", raw, &mut out);
                } else {
                    let spec = raw.trim_matches(['"', '\'', '`']);
                    if !spec.is_empty() {
                        out.push(spec.to_string());
                    }
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
        // Rust: nodes AND use-paths extracted (resolution to files is the store's job).
        let r = analyze(Lang::Rust, "use crate::foo;\npub fn bar() {}");
        assert!(r.nodes.iter().any(|n| n.name == "bar"));
        assert_eq!(r.imports, vec!["crate::foo".to_string()]);
    }

    /// Regression (gauntlet defect `rust-imports-no-ast-dependents`): Rust `use`
    /// paths ARE extracted — groups expand, aliases drop, `self`/`*` collapse to
    /// their prefix, and `pub use` re-exports count. Before this, every Rust
    /// symbol had zero AST-confirmed dependents (`ast_dependents=[]`).
    #[test]
    fn extracts_rust_use_paths() {
        let src = r#"
use crate::modules::brain::gist::{self, Gist};
use super::store::{plan::PlanRow, SqliteIndex as Idx};
use koden_lib::modules::brain::worker::index_dir;
use std::collections::HashMap;
use crate::rank::*;
pub use crate::events::BrainEvent;
use crate::{
    tokenize,
    secrets::redact,
};
"#;
        let a = analyze(Lang::Rust, src);
        for want in [
            "crate::modules::brain::gist",           // {self, …}
            "crate::modules::brain::gist::Gist",
            "super::store::plan::PlanRow",           // nested group
            "super::store::SqliteIndex",             // alias dropped
            "koden_lib::modules::brain::worker::index_dir",
            "std::collections::HashMap",             // extracted; store maps std → no edge
            "crate::rank",                           // wildcard collapses
            "crate::events::BrainEvent",             // pub use re-export
            "crate::tokenize",                       // multi-line group
            "crate::secrets::redact",
        ] {
            assert!(a.imports.contains(&want.to_string()), "missing {want} in {:?}", a.imports);
        }
        // Negative: no alias names, no braces/wildcards leak into specs.
        assert!(!a.imports.iter().any(|s| s.contains('{') || s.contains('*') || s.contains(" as ")),
            "raw syntax leaked: {:?}", a.imports);
        assert!(!a.imports.contains(&"super::store::SqliteIndex as Idx".to_string()));
    }
}
