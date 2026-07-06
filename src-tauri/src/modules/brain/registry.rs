//! `KodenBrainRegistry` — the Brain's project list. Named distinctly from the
//! existing `WorkspaceRegistry` (auth state) to resolve blocker **B6**. Stores
//! root-relative-derived stable ids so the canonical source stays MegaSync-portable
//! (ADR-006 storage model). P0 seeds it from the launch dir; the P1 wizard manages
//! the git-committed canonical source.

use std::path::Path;
use std::sync::RwLock;

use crate::modules::brain::ProjectId;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Project {
    pub id: ProjectId,
    pub name: String,
    /// Absolute, forward-slash-normalized root on this machine.
    pub root: String,
}

/// On-disk source of truth (app-data `workspace.json`). The registry is otherwise
/// in-memory and re-seeded each boot; this makes the project list + the workspace
/// root survive restarts.
#[derive(Default, serde::Serialize, serde::Deserialize)]
struct Persisted {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    workspace_root: Option<String>,
    #[serde(default)]
    projects: Vec<Project>,
}

#[derive(Default)]
pub struct KodenBrainRegistry {
    projects: RwLock<Vec<Project>>,
    /// The user's workspace parent (each child project is registered separately).
    workspace_root: RwLock<Option<String>>,
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

    /// Remove a project by id. Returns the removed project (so a failed downstream
    /// enqueue can [KodenBrainRegistry::restore] it), or `None` if not registered.
    pub fn remove(&self, id: &str) -> Option<Project> {
        let mut guard = self.projects.write().ok()?;
        let idx = guard.iter().position(|p| p.id == id)?;
        Some(guard.remove(idx))
    }

    /// Put back a project taken out by [KodenBrainRegistry::remove] — the rollback
    /// path when the prune couldn't be enqueued. Idempotent by id.
    pub fn restore(&self, project: Project) {
        if let Ok(mut guard) = self.projects.write() {
            if !guard.iter().any(|p| p.id == project.id) {
                guard.push(project);
            }
        }
    }

    /// The configured workspace root (parent of all projects), if any.
    pub fn workspace_root(&self) -> Option<String> {
        self.workspace_root.read().ok().and_then(|g| g.clone())
    }

    /// Set (or clear) the workspace root.
    pub fn set_workspace_root(&self, root: Option<String>) {
        if let Ok(mut g) = self.workspace_root.write() {
            *g = root;
        }
    }

    /// Persist the project list + workspace root to `path` (app-data workspace.json).
    pub fn save_to(&self, path: &Path) {
        let snap = Persisted {
            version: 1,
            workspace_root: self.workspace_root(),
            projects: self.projects(),
        };
        if let Ok(json) = serde_json::to_string_pretty(&snap) {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(path, json);
        }
    }

    /// Load the persisted project list + workspace root (best-effort). Returns true
    /// if a config file was read.
    pub fn load_from(&self, path: &Path) -> bool {
        let Ok(raw) = std::fs::read_to_string(path) else {
            return false;
        };
        let Ok(p) = serde_json::from_str::<Persisted>(&raw) else {
            return false;
        };
        if let Ok(mut g) = self.projects.write() {
            *g = p.projects;
        }
        if let Ok(mut w) = self.workspace_root.write() {
            *w = p.workspace_root;
        }
        true
    }

    /// Longest-prefix match `cwd` → project (CONCEPT §5.2; used by P3 gist
    /// resolution). Picks the most specific (longest root) when projects nest.
    /// Case-folded on Windows (see [fold_case]) — a shell reporting `c:\work\repo`
    /// must still resolve against a stored `C:/Work/Repo` root, mirroring the
    /// frontend's `resolveProjectForCwd`.
    pub fn resolve(&self, cwd: &str) -> Option<Project> {
        let cwd_n = fold_case(&normalize(Path::new(cwd)));
        let guard = self.projects.read().ok()?;
        guard
            .iter()
            .filter(|p| {
                let root = fold_case(&p.root);
                cwd_n == root || cwd_n.starts_with(&format!("{root}/"))
            })
            .max_by_key(|p| p.root.len())
            .cloned()
    }
}

/// Case-fold a canonical path for identity/prefix comparison. Windows filesystems
/// are case-insensitive, so folding is required there; Unix paths are
/// case-SENSITIVE, so folding would collide legitimately-distinct roots
/// (e.g. `/a/Foo` vs `/a/foo`).
/// ponytail: macOS APFS defaults to case-insensitive yet is NOT folded here
/// (canonicalize normalizes to on-disk casing, so spellings converge in practice);
/// if a real case-spelling miss surfaces on macOS, widen the predicate to
/// `cfg!(any(windows, target_os = "macos"))`.
pub(crate) fn fold_case(s: &str) -> String {
    if cfg!(windows) {
        s.to_lowercase()
    } else {
        s.to_string()
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

/// Stable 16-hex-char id derived from the canonical root. Case-folded on Windows
/// ONLY (same [fold_case] predicate as `resolve`): folding on Unix would collide
/// case-differing roots (`/a/Foo` and `/a/foo` are distinct directories there).
/// Windows ids are unchanged from the previous unconditional-lowercase derivation.
fn project_id_for(root_str: &str) -> ProjectId {
    blake3::hash(fold_case(root_str).as_bytes()).to_hex()[..16].to_string()
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

    /// ADR-010 cluster 7: Windows cwd casing must not break resolution — a shell
    /// reporting `c:\work\repo` still matches a stored `C:/Work/Repo` root.
    #[cfg(windows)]
    #[test]
    fn resolve_folds_case_on_windows() {
        let reg = KodenBrainRegistry::default();
        {
            let mut g = reg.projects.write().unwrap();
            g.push(Project { id: "p".into(), name: "p".into(), root: "C:/Work/Repo".into() });
        }
        assert_eq!(reg.resolve(r"c:\WORK\repo\src\main.rs").unwrap().id, "p");
        assert_eq!(reg.resolve("c:/work/repo").unwrap().id, "p");
        // Ids fold the same way: both spellings derive the same stable id.
        assert_eq!(project_id_for("C:/Work/Repo"), project_id_for("c:/work/repo"));
    }

    /// Unix paths are case-sensitive: case-differing roots are DISTINCT projects
    /// and must not collide on one id (the pre-fix unconditional lowercase did).
    #[cfg(not(windows))]
    #[test]
    fn project_ids_do_not_fold_case_on_unix() {
        assert_ne!(project_id_for("/a/Foo"), project_id_for("/a/foo"));
    }

    #[test]
    fn remove_returns_project_and_restore_reinserts() {
        let reg = KodenBrainRegistry::default();
        let dir = tempfile::tempdir().unwrap();
        let p = reg.add_root(dir.path()).unwrap();
        let removed = reg.remove(&p.id).expect("registered project removed");
        assert_eq!(removed.id, p.id);
        assert!(reg.projects().is_empty());
        assert!(reg.remove(&p.id).is_none(), "second remove is a no-op");
        // Rollback path: restore puts the exact project back (idempotent by id).
        reg.restore(removed.clone());
        reg.restore(removed);
        assert_eq!(reg.projects().len(), 1);
        assert_eq!(reg.projects()[0].id, p.id);
    }
}
