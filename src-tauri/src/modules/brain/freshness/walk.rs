//! Gitignore-aware initial population via `ignore::WalkBuilder` (already in-tree).
//! Bounded so a cursed/huge repo degrades gracefully and never freezes the UI
//! (CONCEPT §8, BUILD-PROMPT §13.14). The secrets file-denylist is applied at the
//! door so credential files are never even read.

use std::path::{Path, PathBuf};

use ignore::WalkBuilder;

use crate::modules::brain::secrets;

/// Cap on directory entries scanned per project (mirrors `fs/search.rs` bounds).
pub const MAX_SCANNED: usize = 50_000;
/// Per-file size cap for indexing (CONCEPT §8 [DP-27]: "skip > 1 MB"). The Brain's
/// own cap — do NOT confuse with `git/types.rs` MAX_FILE_BYTES=2MB.
pub const MAX_INDEX_FILE_BYTES: u64 = 1024 * 1024;

/// Base directories never indexed even if not gitignored (CONCEPT §8,
/// BUILD-PROMPT §13.8). `.git` is also handled by the ignore walker.
const BASE_SKIP_DIRS: &[&str] = &[
    "node_modules", ".git", "dist", "build", "target", ".next", ".turbo",
    "coverage", ".venv", "venv", "vendor", "generated", ".cache", ".svelte-kit",
];

/// True if any component of `path` is a base-skip dir. Used by the incremental
/// watcher path, which gets absolute changed paths with no project-relative
/// context (the full walk uses `in_skip_dir` with the root).
pub fn under_skip_dir(path: &Path) -> bool {
    path.components().any(|c| {
        matches!(c, std::path::Component::Normal(os)
            if os.to_str().is_some_and(|s| BASE_SKIP_DIRS.contains(&s)))
    })
}

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

/// Outcome of a bounded walk. `complete == false` means the view is PARTIAL —
/// the scan cap truncated traversal or a directory was unreadable — and MUST
/// never feed reconcile-delete (ADR-010: deletion needs positive evidence of
/// absence, never inference from a truncated walk or a read failure; otherwise
/// files past the cap oscillate: pruned each full pass, re-indexed by the watcher).
pub struct Walked {
    pub files: Vec<PathBuf>,
    pub complete: bool,
}

/// Walk a project root and return indexable candidate files. Honors
/// `.gitignore`/`.kodenignore`, skips base-denied dirs + secret-denylisted files
/// + oversized files, and stops after `MAX_SCANNED` entries (flagged as
/// incomplete on the returned [Walked]).
pub fn walk_files(root: &Path) -> Walked {
    walk_files_capped(root, MAX_SCANNED)
}

fn walk_files_capped(root: &Path, cap: usize) -> Walked {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut complete = true;
    let mut scanned = 0usize;
    let walker = WalkBuilder::new(root)
        .standard_filters(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .hidden(false) // index dotfiles (e.g. config); the denylist guards secrets
        .follow_links(false)
        .add_custom_ignore_filename(".kodenignore")
        // Prune heavy dirs during traversal (not just post-yield) so a
        // non-gitignored node_modules is never descended — mirrors fs/search.rs.
        .filter_entry(|dent| {
            dent.depth() == 0
                || dent
                    .file_name()
                    .to_str()
                    .map(|n| !BASE_SKIP_DIRS.contains(&n))
                    .unwrap_or(true)
        })
        .build();
    for entry in walker {
        scanned += 1;
        if scanned > cap {
            log::warn!("brain: walk hit scan cap ({cap}) at {}", root.display());
            complete = false;
            break;
        }
        let entry = match entry {
            Ok(e) => e,
            Err(err) => {
                // An IO error (unreadable dir/entry) hides an unknown subtree —
                // the walk is PARTIAL. Non-IO errors (e.g. a malformed ignore
                // glob) don't hide files, so they don't taint completeness.
                if err.io_error().is_some() {
                    log::warn!("brain: walk error under {} ({err}); pass is partial", root.display());
                    complete = false;
                }
                continue;
            }
        };
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
        match std::fs::metadata(&path) {
            Ok(m) if m.len() > MAX_INDEX_FILE_BYTES => continue, // oversized — present but not indexable
            Ok(_) => {}
            // Stat failed (lock/permission blip): state UNKNOWN, not absent —
            // yield it and let the bounded read path classify it (ADR-010).
            Err(_) => {}
        }
        out.push(path);
    }
    Walked { files: out, complete }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capped_walk_is_flagged_partial() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..5 {
            std::fs::write(dir.path().join(format!("f{i}.txt")), b"x").unwrap();
        }
        let full = walk_files_capped(dir.path(), MAX_SCANNED);
        assert!(full.complete, "an uncapped walk is complete");
        assert_eq!(full.files.len(), 5);
        // A cap smaller than the entry count (root dir + 5 files) truncates.
        let partial = walk_files_capped(dir.path(), 3);
        assert!(!partial.complete, "a cap-hit walk must be flagged PARTIAL");
        assert!(partial.files.len() < 5, "truncated walk yields fewer files");
    }
}
