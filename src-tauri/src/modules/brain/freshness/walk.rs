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
    // Agent-tooling state: leftover Claude-Code worktrees under .claude/worktrees/
    // are full stale copies of the repo — indexing them duplicates every symbol
    // (gauntlet defect claude-worktrees-indexed: 978/1605 rows were copies).
    ".claude",
];

/// True if any `/`-separated component of a PROJECT-RELATIVE path (as produced
/// by the worker's `rel_path`) is a base-skip dir. Used by the incremental
/// watcher path. Deliberately takes the RELATIVE form: checking the ABSOLUTE
/// path would skip every update for a project that itself lives under a dir
/// named e.g. `build/` or `vendor/` (the full walk uses `in_skip_dir` with the
/// root for the same reason).
pub fn rel_under_skip_dir(rel: &str) -> bool {
    rel.split('/').any(|c| BASE_SKIP_DIRS.contains(&c))
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
    walk_files_capped(root, root, MAX_SCANNED)
}

/// Walk a SUBTREE of a project (the watcher's dir-event path) with the same
/// ignore context as the full walk: in-project ancestor `.gitignore`/
/// `.kodenignore` files between `root` and `start` are replayed explicitly,
/// because with parent traversal bounded (see below) the walker only reads
/// ignore files at/under its start dir.
pub fn walk_files_under(root: &Path, start: &Path) -> Walked {
    walk_files_capped(root, start, MAX_SCANNED)
}

/// In-project ancestor dirs of `start`, from `root` (inclusive) down to
/// `start`'s parent (inclusive), root-first. Empty if `start` is not under `root`.
fn ancestor_chain(root: &Path, start: &Path) -> Vec<PathBuf> {
    let Ok(rel) = start.strip_prefix(root) else {
        return Vec::new();
    };
    let mut out = vec![root.to_path_buf()];
    let mut cur = root.to_path_buf();
    let comps: Vec<std::path::Component> = rel.components().collect();
    for comp in comps.iter().take(comps.len().saturating_sub(1)) {
        cur.push(comp);
        out.push(cur.clone());
    }
    out
}

fn walk_files_capped(root: &Path, start: &Path, cap: usize) -> Walked {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut complete = true;
    let mut scanned = 0usize;
    let mut builder = WalkBuilder::new(start);
    builder
        .standard_filters(true)
        .git_ignore(true)
        // CONCEPT §7.1: ignore rules apply to git AND non-git projects uniformly —
        // the crate's default require_git(true) made .gitignore dead in non-git roots.
        .require_git(false)
        // …but bound ignore-file discovery to the project: with require_git off,
        // parent traversal would pull ancestor .gitignores from ABOVE the project
        // root and over-ignore. In-project ancestors are replayed below for the
        // subtree (dir-event) walk.
        // ponytail: a project registered as a subdir of a bigger git repo no longer
        // sees that repo's root .gitignore; upgrade path = replay root..git-root.
        .parents(false)
        // Exactly the sources CONCEPT §7.1 names (.gitignore + .kodenignore) and
        // exactly the sources `is_ignored_file` replays. Any source honored here
        // but not by the watcher gate re-opens the index/prune oscillation this
        // module exists to close — with require_git(false) the crate would apply
        // the user's GLOBAL gitignore (e.g. Thumbs.db) even to non-git roots,
        // which the gate never sees: watcher indexes, full pass prunes, repeat.
        // ponytail: global gitignore / .git/info/exclude / .ignore files are
        // deliberately unsupported; upgrade path = add the source to BOTH this
        // builder and `is_ignored_file`, never one side.
        .git_global(false)
        .git_exclude(false)
        .ignore(false)
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
        });
    if start != root {
        // Dir-event walk: replay ancestor ignore files (root-first; `start`'s own
        // files the walker reads itself) so the subtree walk agrees with the full
        // walk. `add_ignore` is lowest-precedence, matching git's deeper-file-wins
        // for the common cases.
        // ponytail: the replay is an approximation — a replayed ancestor
        // .kodenignore ranks BELOW a per-dir .gitignore here, so cross-source
        // precedence can over-yield (deeper `!negation` un-ignores a
        // .kodenignore'd file) or under-yield vs the full walk. The worker
        // fronts every yielded child with `is_ignored_file` (exact precedence),
        // so over-yield never reaches the index; under-yield self-heals on the
        // next full pass. Exact fix = per-ancestor custom-ignore replay.
        for dir in ancestor_chain(root, start) {
            for name in [".gitignore", ".kodenignore"] {
                let f = dir.join(name);
                if f.is_file() {
                    if let Some(e) = builder.add_ignore(&f) {
                        log::debug!("brain: unparsable ignore file {} ({e})", f.display());
                    }
                }
            }
        }
    }
    let walker = builder.build();
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

/// Per-file ignore check for the incremental watcher path: true if `path`
/// (under `root`) is excluded by in-project `.gitignore`/`.kodenignore` rules —
/// so a watch event for an ignored file agrees with the full walk, which never
/// yields it (otherwise the file would be indexed by the watcher and pruned by
/// the next full pass, oscillating). Semantics mirror the walk: only ignore
/// files at/below the project root count, a `.kodenignore` match at ANY depth
/// outranks a `.gitignore` match at ANY depth (the crate's custom-ignore
/// precedence, `m_custom_ignore.or(m_gi)`), within a source the deeper file
/// wins, and an ignored ancestor DIR ignores everything below it (the walker
/// never descends into one). The walk honors exactly these two sources too —
/// git-global / .git/info/exclude / .ignore are all disabled on the builder —
/// so gate and walk agree on every file.
pub fn is_ignored_file(root: &Path, path: &Path) -> bool {
    use ignore::gitignore::Gitignore;
    let Ok(rel) = path.strip_prefix(root) else {
        return false;
    };
    // One stack per SOURCE: the walker resolves each source independently
    // (deepest file wins within a source), then lets any .kodenignore verdict
    // outrank any .gitignore verdict — a single mixed stack would instead let a
    // deeper .gitignore negation override a shallower .kodenignore rule.
    fn push_dir(dir: &Path, git: &mut Vec<Gitignore>, koden: &mut Vec<Gitignore>) {
        let g = dir.join(".gitignore");
        if g.is_file() {
            git.push(Gitignore::new(g).0); // parse errors → partial matcher, same as the walker
        }
        let k = dir.join(".kodenignore");
        if k.is_file() {
            koden.push(Gitignore::new(k).0);
        }
    }
    let mut git_stack: Vec<Gitignore> = Vec::new();
    let mut koden_stack: Vec<Gitignore> = Vec::new();
    let mut dir = root.to_path_buf();
    push_dir(&dir, &mut git_stack, &mut koden_stack);
    let comps: Vec<std::path::Component> = rel.components().collect();
    for (i, comp) in comps.iter().enumerate() {
        let is_last = i + 1 == comps.len();
        dir.push(comp);
        // Live is_dir for the leaf: a dir event for an ignored dir must gate
        // (its children are never walked), while a DELETED path stats false and
        // sails through to the prune branch.
        let is_dir = if is_last { path.is_dir() } else { true };
        // Some(true)=ignore, Some(false)=whitelist, None=no rule in this source.
        let per_source = |stack: &[Gitignore]| -> Option<bool> {
            stack
                .iter()
                .rev() // deepest matcher wins within a source (git semantics)
                .map(|g| g.matched(&dir, is_dir))
                .find(|m| !m.is_none())
                .map(|m| m.is_ignore())
        };
        if per_source(&koden_stack).or_else(|| per_source(&git_stack)) == Some(true) {
            return true;
        }
        if !is_last {
            push_dir(&dir, &mut git_stack, &mut koden_stack);
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ADR-010 cluster 2: the skip gate is over PROJECT-RELATIVE components —
    /// a skip-named dir in the part of the path ABOVE the project root (which
    /// `rel_under_skip_dir` never sees) must not blank out updates.
    #[test]
    fn skip_gate_is_project_relative() {
        assert!(rel_under_skip_dir("node_modules/pkg/index.js"));
        assert!(rel_under_skip_dir("src/dist/bundle.js")); // nested skip dir
        assert!(!rel_under_skip_dir("src/main.rs"));
        assert!(!rel_under_skip_dir("distx/main.rs")); // component match, not substring
        assert!(!rel_under_skip_dir(""));
    }

    /// Gauntlet defect claude-worktrees-indexed: leftover Claude-Code agent
    /// worktrees under `.claude/worktrees/` are full stale copies of the repo.
    /// Indexing them duplicated 61% of rows and tripled code_impact results.
    /// `.claude` must be base-skipped on BOTH paths: the full walk (in_skip_dir
    /// + filter_entry prune) and the watcher gate (rel_under_skip_dir).
    #[test]
    fn claude_worktrees_are_never_indexed() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("src");
        std::fs::create_dir_all(&real).unwrap();
        std::fs::write(real.join("redact.ts"), b"export function redactSensitive() {}").unwrap();
        // Stale worktree copy of the same file, as an agent leaves it behind.
        let wt = dir
            .path()
            .join(".claude")
            .join("worktrees")
            .join("agent-a2b92c098ab5ffd64")
            .join("src");
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::write(wt.join("redact.ts"), b"export function redactSensitive() {}").unwrap();
        let got = &walk_files(dir.path()).files;
        assert_eq!(got.len(), 1, "only the real tree's file is yielded, got {got:?}");
        assert!(got[0].starts_with(&real), "yielded file is the real one, not the worktree copy");
        // Watcher path agrees (a save inside a live agent worktree must not index).
        assert!(rel_under_skip_dir(".claude/worktrees/agent-x/src/redact.ts"));
        assert!(rel_under_skip_dir(".claude/settings.json"));
        // Negative control: a dir merely NAMED like it is not skipped.
        assert!(!rel_under_skip_dir("src/claude/notes.md"));
        assert!(!rel_under_skip_dir("claude-tools/x.ts"));
    }

    #[test]
    fn capped_walk_is_flagged_partial() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..5 {
            std::fs::write(dir.path().join(format!("f{i}.txt")), b"x").unwrap();
        }
        let full = walk_files_capped(dir.path(), dir.path(), MAX_SCANNED);
        assert!(full.complete, "an uncapped walk is complete");
        assert_eq!(full.files.len(), 5);
        // A cap smaller than the entry count (root dir + 5 files) truncates.
        let partial = walk_files_capped(dir.path(), dir.path(), 3);
        assert!(!partial.complete, "a cap-hit walk must be flagged PARTIAL");
        assert!(partial.files.len() < 5, "truncated walk yields fewer files");
    }

    fn names(w: &Walked) -> Vec<String> {
        let mut v: Vec<String> = w
            .files
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    /// CONCEPT §7.1: ignore rules apply uniformly — a NON-git project root's
    /// .gitignore (and .kodenignore) must take effect, not require a .git dir.
    #[test]
    fn gitignore_honored_in_non_git_root() {
        let dir = tempfile::tempdir().unwrap(); // no .git anywhere under it
        std::fs::write(dir.path().join(".gitignore"), "zz_gitignored.txt\n").unwrap();
        std::fs::write(dir.path().join(".kodenignore"), "zz_kodenignored.txt\n").unwrap();
        std::fs::write(dir.path().join("zz_gitignored.txt"), b"x").unwrap();
        std::fs::write(dir.path().join("zz_kodenignored.txt"), b"x").unwrap();
        std::fs::write(dir.path().join("zz_kept.txt"), b"x").unwrap();
        let got = names(&walk_files(dir.path()));
        assert!(!got.contains(&"zz_gitignored.txt".into()), ".gitignore'd file must not be yielded");
        assert!(!got.contains(&"zz_kodenignored.txt".into()), ".kodenignore'd file must not be yielded");
        assert!(got.contains(&"zz_kept.txt".into()), "sibling IS yielded");
    }

    /// Unchanged in a git root: .gitignore still honored (require_git(false) is
    /// a superset of the old behavior when a .git dir exists).
    #[test]
    fn gitignore_still_honored_in_git_root() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap(); // repo marker
        std::fs::write(dir.path().join(".gitignore"), "zz_gitignored.txt\n").unwrap();
        std::fs::write(dir.path().join("zz_gitignored.txt"), b"x").unwrap();
        std::fs::write(dir.path().join("zz_kept.txt"), b"x").unwrap();
        let got = names(&walk_files(dir.path()));
        assert!(!got.contains(&"zz_gitignored.txt".into()));
        assert!(got.contains(&"zz_kept.txt".into()));
    }

    /// Over-ignoring guard: a .gitignore ABOVE the project root has no effect —
    /// only in-project ignore files count (parents(false)).
    #[test]
    fn ancestor_gitignore_above_root_has_no_effect() {
        let outer = tempfile::tempdir().unwrap();
        std::fs::write(outer.path().join(".gitignore"), "zz_shadowed.txt\n").unwrap();
        let root = outer.path().join("proj");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("zz_shadowed.txt"), b"x").unwrap();
        let got = names(&walk_files(&root));
        assert!(
            got.contains(&"zz_shadowed.txt".into()),
            "an out-of-project ancestor .gitignore must not leak into the walk"
        );
    }

    /// The dir-event subtree walk replays in-project ancestor ignore files, so
    /// it yields exactly what the full walk yields for that subtree.
    #[test]
    fn subtree_walk_agrees_with_full_walk() {
        let dir = tempfile::tempdir().unwrap(); // non-git project root
        std::fs::write(dir.path().join(".gitignore"), "*.zzgen\n").unwrap();
        let sub = dir.path().join("moved");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("a.zzgen"), b"x").unwrap();
        std::fs::write(sub.join("a.txt"), b"x").unwrap();
        let subtree = names(&walk_files_under(dir.path(), &sub));
        assert_eq!(subtree, vec!["a.txt".to_string()], "ancestor .gitignore applies to the subtree walk");
        let full: Vec<String> = walk_files(dir.path())
            .files
            .iter()
            .filter(|p| p.starts_with(&sub))
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(subtree, full, "dir-event path and full walk must agree");
    }

    /// Gate/walk agreement on SOURCES: only .gitignore + .kodenignore count.
    /// A file ignored solely by .git/info/exclude or a `.ignore` file must be
    /// yielded by the walk (the gate can't replay those sources; honoring them
    /// on one side only would oscillate: watcher-indexed, full-pass-pruned).
    #[test]
    fn walk_ignores_only_the_sources_the_gate_replays() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join(".git").join("info")).unwrap();
        std::fs::write(root.join(".git").join("info").join("exclude"), "zz_excl.txt\n").unwrap();
        std::fs::write(root.join(".ignore"), "zz_dotign.txt\n").unwrap();
        std::fs::write(root.join("zz_excl.txt"), b"x").unwrap();
        std::fs::write(root.join("zz_dotign.txt"), b"x").unwrap();
        let got = names(&walk_files(root));
        assert!(got.contains(&"zz_excl.txt".into()), ".git/info/exclude is not a Koden ignore source");
        assert!(got.contains(&"zz_dotign.txt".into()), ".ignore files are not a Koden ignore source");
        // …and the gate agrees: neither file is ignored there either.
        assert!(!is_ignored_file(root, &root.join("zz_excl.txt")));
        assert!(!is_ignored_file(root, &root.join("zz_dotign.txt")));
    }

    /// Cross-depth precedence agreement: a root .kodenignore rule beats a
    /// DEEPER .gitignore negation in the walker (custom-ignore matches at any
    /// depth outrank .gitignore matches at any depth), so the gate must reach
    /// the same verdict — a mixed deepest-first stack would let the negation
    /// win in the gate only, and the file would oscillate.
    #[test]
    fn kodenignore_outranks_deeper_gitignore_negation_in_walk_and_gate() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let sub = root.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(root.join(".kodenignore"), "zz.txt\n").unwrap();
        std::fs::write(sub.join(".gitignore"), "!zz.txt\n").unwrap();
        std::fs::write(sub.join("zz.txt"), b"x").unwrap();
        let got = names(&walk_files(root));
        assert!(!got.contains(&"zz.txt".into()), "walker: .kodenignore outranks deeper !negation");
        assert!(is_ignored_file(root, &sub.join("zz.txt")), "gate must agree with the walker");
        // Sanity: within .gitignore alone, the deeper negation DOES win.
        std::fs::write(root.join(".gitignore"), "zz_g.txt\n").unwrap();
        std::fs::write(sub.join("zz_g.txt"), b"x").unwrap();
        std::fs::write(sub.join(".gitignore"), "!zz.txt\n!zz_g.txt\n").unwrap();
        let got = names(&walk_files(root));
        assert!(got.contains(&"zz_g.txt".into()), "walker: deeper .gitignore negation wins in-source");
        assert!(!is_ignored_file(root, &sub.join("zz_g.txt")), "gate must agree in-source too");
    }

    /// The watcher's per-file gate agrees with the walk: root + nested ignore
    /// files, .kodenignore, ignored ancestor dirs, and the ancestor bound.
    #[test]
    fn per_file_check_agrees_with_the_walk() {
        let outer = tempfile::tempdir().unwrap();
        std::fs::write(outer.path().join(".gitignore"), "zz_above.txt\n").unwrap();
        let root = outer.path().join("proj");
        let sub = root.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(root.join(".gitignore"), "*.zzgen\nzzdir/\n").unwrap();
        std::fs::write(root.join(".kodenignore"), "zz_koden.txt\n").unwrap();
        std::fs::write(sub.join(".gitignore"), "local.txt\n").unwrap();
        assert!(is_ignored_file(&root, &root.join("a.zzgen")));
        assert!(is_ignored_file(&root, &sub.join("b.zzgen")), "root pattern reaches subdirs");
        assert!(is_ignored_file(&root, &root.join("zz_koden.txt")), ".kodenignore honored");
        assert!(is_ignored_file(&root, &sub.join("local.txt")), "nested .gitignore honored");
        assert!(!is_ignored_file(&root, &root.join("local.txt")), "nested rule stays in its subtree");
        assert!(!is_ignored_file(&root, &sub.join("kept.txt")));
        assert!(
            is_ignored_file(&root, &root.join("zzdir").join("under.txt")),
            "file under an ignored dir is ignored (walker never descends)"
        );
        assert!(
            !is_ignored_file(&root, &root.join("zz_above.txt")),
            "out-of-project ancestor .gitignore must not leak into the gate"
        );
        assert!(!is_ignored_file(&root, std::path::Path::new("/elsewhere/x.txt")), "outside root → not ours to judge");
    }
}
