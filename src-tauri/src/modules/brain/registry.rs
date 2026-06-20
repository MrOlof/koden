//! `KodenBrainRegistry` — the Brain's project list. Named distinctly from the
//! existing `WorkspaceRegistry` (auth state) to resolve blocker **B6**. Stores
//! root-relative-derived stable ids so the canonical source stays MegaSync-portable
//! (ADR-006 storage model). P0 seeds it from the launch dir; the P1 wizard manages
//! the git-committed canonical source.

use std::path::Path;
use std::sync::RwLock;

use crate::modules::brain::ProjectId;

#[derive(Clone, Debug, serde::Serialize)]
pub struct Project {
    pub id: ProjectId,
    pub name: String,
    /// Absolute, forward-slash-normalized root on this machine.
    pub root: String,
}

#[derive(Default)]
pub struct KodenBrainRegistry {
    projects: RwLock<Vec<Project>>,
}

impl KodenBrainRegistry {
    pub fn projects(&self) -> Vec<Project> {
        self.projects.read().map(|p| p.clone()).unwrap_or_default()
    }

    /// Register a project root (idempotent by stable id). Returns the project.
    pub fn add_root(&self, root: &Path) -> Option<Project> {
        let canon = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
        let root_str = normalize(&canon);
        if root_str.is_empty() {
            return None;
        }
        let id = project_id_for(&root_str);
        let name = canon
            .file_name()
            .and_then(|s| s.to_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("project")
            .to_string();
        let mut guard = self.projects.write().ok()?;
        if let Some(p) = guard.iter().find(|p| p.id == id) {
            return Some(p.clone());
        }
        let p = Project { id, name, root: root_str };
        guard.push(p.clone());
        Some(p)
    }

    /// Longest-prefix match `cwd` → project (CONCEPT §5.2; used by P3 gist
    /// resolution). Picks the most specific (longest root) when projects nest.
    pub fn resolve(&self, cwd: &str) -> Option<Project> {
        let cwd_n = normalize(Path::new(cwd));
        let guard = self.projects.read().ok()?;
        guard
            .iter()
            .filter(|p| cwd_n == p.root || cwd_n.starts_with(&format!("{}/", p.root)))
            .max_by_key(|p| p.root.len())
            .cloned()
    }
}

fn normalize(p: &Path) -> String {
    // Route through `to_canon` so the Windows `\\?\` verbatim prefix is STRIPPED
    // (not just backslash-swapped). The watcher's `group_by_project` and the
    // worker's `rel_path` both canonicalize paths via `to_canon`; the stored root
    // MUST use the same form or longest-prefix matching silently fails on Windows
    // (the incremental watcher goes dead and full-vs-incremental rel keys diverge).
    crate::modules::fs::to_canon(p)
        .trim_end_matches('/')
        .to_string()
}

/// Stable 16-hex-char id derived from the canonical root (case-insensitive).
fn project_id_for(root_str: &str) -> ProjectId {
    blake3::hash(root_str.to_lowercase().as_bytes()).to_hex()[..16].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_is_idempotent() {
        let reg = KodenBrainRegistry::default();
        let dir = tempfile::tempdir().unwrap();
        let a = reg.add_root(dir.path()).unwrap();
        let b = reg.add_root(dir.path()).unwrap();
        assert_eq!(a.id, b.id);
        assert_eq!(reg.projects().len(), 1);
    }

    #[cfg(windows)]
    #[test]
    fn normalize_strips_windows_verbatim_prefix() {
        // The bug: a `\\?\C:\...` root stored as `//?/C:/...` never matches a
        // `to_canon`'d `C:/...` change path. `to_canon` must strip the prefix.
        let n = normalize(Path::new(r"\\?\C:\Users\x\repo"));
        assert_eq!(n, "C:/Users/x/repo");
        assert!(!n.contains('?'), "verbatim prefix must be gone: {n}");
    }

    #[test]
    fn resolve_longest_prefix() {
        let reg = KodenBrainRegistry::default();
        // simulate nested registered roots
        {
            let mut g = reg.projects.write().unwrap();
            g.push(Project { id: "outer".into(), name: "o".into(), root: "/work/repo".into() });
            g.push(Project { id: "inner".into(), name: "i".into(), root: "/work/repo/pkg".into() });
        }
        assert_eq!(reg.resolve("/work/repo/pkg/src/main.rs").unwrap().id, "inner");
        assert_eq!(reg.resolve("/work/repo/README.md").unwrap().id, "outer");
        assert!(reg.resolve("/elsewhere").is_none());
    }
}
