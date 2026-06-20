//! Gitignore-aware initial population via `ignore::WalkBuilder` (already in-tree).
//! Bounded so a cursed/huge repo degrades gracefully and never freezes the UI
//! (CONCEPT §8, BUILD-PROMPT §13.14). The secrets file-denylist is applied at the
//! door so credential files are never even read.

use std::path::{Path, PathBuf};

use ignore::WalkBuilder;

use crate::modules::brain::secrets;

/// Cap on directory entries scanned per project (mirrors `fs/search.rs` bounds).
pub const MAX_SCANNED: usize = 50_000;
/// Per-file size cap for indexing — the Brain's own cap (a new invention per
/// EXECUTION_PLAN §0.2; do NOT confuse with `git/types.rs` MAX_FILE_BYTES=2MB).
pub const MAX_INDEX_FILE_BYTES: u64 = 256 * 1024;

/// Base directories never indexed even if not gitignored (CONCEPT §8,
/// BUILD-PROMPT §13.8). `.git` is also handled by the ignore walker.
const BASE_SKIP_DIRS: &[&str] = &[
    "node_modules", ".git", "dist", "build", "target", ".next", ".turbo",
    "coverage", ".venv", "venv", "vendor", "generated", ".cache", ".svelte-kit",
];

fn in_skip_dir(path: &Path, root: &Path) -> bool {
    let Ok(rel) = path.strip_prefix(root) else {
        return false;
    };
    for comp in rel.components() {
        if let std::path::Component::Normal(os) = comp {
            if let Some(s) = os.to_str() {
                if BASE_SKIP_DIRS.contains(&s) {
                    return true;
                }
            }
        }
    }
    false
}

/// Walk a project root and return indexable candidate files. Honors
/// `.gitignore`/`.kodenignore`, skips base-denied dirs + secret-denylisted files
/// + oversized files, and stops after `MAX_SCANNED` entries.
pub fn walk_files(root: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut scanned = 0usize;
    let walker = WalkBuilder::new(root)
        .standard_filters(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .hidden(false) // index dotfiles (e.g. config); the denylist guards secrets
        .follow_links(false)
        .add_custom_ignore_filename(".kodenignore")
        .build();
    for entry in walker {
        scanned += 1;
        if scanned > MAX_SCANNED {
            log::warn!("brain: walk hit MAX_SCANNED ({MAX_SCANNED}) at {}", root.display());
            break;
        }
        let Ok(entry) = entry else { continue };
        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
        let path = entry.into_path();
        if in_skip_dir(&path, root) {
            continue;
        }
        if secrets::is_denylisted_path(&path.to_string_lossy()) {
            continue;
        }
        if std::fs::metadata(&path)
            .map(|m| m.len() > MAX_INDEX_FILE_BYTES)
            .unwrap_or(true)
        {
            continue;
        }
        out.push(path);
    }
    out
}
