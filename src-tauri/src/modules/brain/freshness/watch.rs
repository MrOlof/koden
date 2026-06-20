//! Brain-owned RECURSIVE `notify` watcher (CONCEPT §5.2, ADR-006). The existing
//! `fs/watch.rs` is NonRecursive/per-open-dir and unsuitable; the brain watches
//! whole project roots. Events are debounced + coalesced (reusing fs/watch.rs's
//! 150ms/1000ms constants), resolved to their project (longest-prefix over the
//! canonical roots, like `registry.resolve`), and fed to the worker as
//! `BrainEvent::Fs`. The worker then incrementally reindexes ONLY the changed
//! files — the "warm, incrementally-fresh" thesis over Conductr's cold rehash.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::modules::brain::events::BrainEvent;
use crate::modules::brain::ProjectId;
use crate::modules::fs::to_canon;

const DEBOUNCE: Duration = Duration::from_millis(150);
const MAX_WINDOW: Duration = Duration::from_millis(1000);

/// Arm a recursive watcher over each project root. Returns the watcher handle —
/// the caller MUST keep it alive (dropping it stops watching). `projects` is
/// `(id, canonical-root-string)`. Fail-open: `None` if the watcher can't start.
pub fn spawn(
    projects: Vec<(ProjectId, String)>,
    tx: Sender<BrainEvent>,
) -> Option<RecommendedWatcher> {
    if projects.is_empty() {
        return None;
    }
    let (raw_tx, raw_rx) = mpsc::channel::<notify::Result<Event>>();
    let mut watcher = match RecommendedWatcher::new(
        move |res| {
            let _ = raw_tx.send(res);
        },
        Config::default(),
    ) {
        Ok(w) => w,
        Err(e) => {
            log::warn!("brain: watcher create failed: {e}");
            return None;
        }
    };
    for (_, root) in &projects {
        if let Err(e) = watcher.watch(Path::new(root), RecursiveMode::Recursive) {
            log::warn!("brain: watch '{root}' failed: {e}");
        }
    }
    if let Err(e) = std::thread::Builder::new()
        .name("koden-brain-watch".into())
        .spawn(move || drain_loop(raw_rx, projects, tx))
    {
        log::warn!("brain: watch drain spawn failed: {e}");
        return None; // watcher drops → stops watching (fail-open)
    }
    Some(watcher)
}

fn drain_loop(
    rx: mpsc::Receiver<notify::Result<Event>>,
    projects: Vec<(ProjectId, String)>,
    tx: Sender<BrainEvent>,
) {
    loop {
        let first = match rx.recv() {
            Ok(e) => e,
            Err(_) => return,
        };
        let mut paths: HashSet<PathBuf> = HashSet::new();
        collect(&mut paths, first);

        // Coalesce a burst: quiet-gap of DEBOUNCE, capped at MAX_WINDOW — so a
        // save-all / git pull collapses into one delta per project.
        let deadline = Instant::now() + MAX_WINDOW;
        loop {
            let timeout = DEBOUNCE.min(deadline.saturating_duration_since(Instant::now()));
            match rx.recv_timeout(timeout) {
                Ok(e) => collect(&mut paths, e),
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return,
            }
            if Instant::now() >= deadline {
                break;
            }
        }
        if paths.is_empty() {
            continue;
        }
        for (project, changed) in group_by_project(&projects, paths) {
            if tx.send(BrainEvent::Fs { project, changed }).is_err() {
                return; // worker gone
            }
        }
    }
}

fn collect(set: &mut HashSet<PathBuf>, ev: notify::Result<Event>) {
    let Ok(ev) = ev else { return };
    // Access (reads) never change content.
    if matches!(ev.kind, EventKind::Access(_)) {
        return;
    }
    for p in ev.paths {
        set.insert(p);
    }
}

/// Group changed paths by project via longest-prefix over the canonical roots
/// (mirrors `registry.resolve`). Pure — unit-tested deterministically.
pub fn group_by_project(
    projects: &[(ProjectId, String)],
    paths: HashSet<PathBuf>,
) -> HashMap<ProjectId, Vec<PathBuf>> {
    let mut out: HashMap<ProjectId, Vec<PathBuf>> = HashMap::new();
    for p in paths {
        let pc = to_canon(&p);
        let best = projects
            .iter()
            .filter(|(_, root)| pc == *root || pc.starts_with(&format!("{root}/")))
            .max_by_key(|(_, root)| root.len());
        if let Some((id, _)) = best {
            out.entry(id.clone()).or_default().push(p);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_by_longest_prefix_and_drops_outsiders() {
        let projects = vec![
            ("outer".to_string(), "/work/repo".to_string()),
            ("inner".to_string(), "/work/repo/pkg".to_string()),
        ];
        let mut paths = HashSet::new();
        paths.insert(PathBuf::from("/work/repo/src/main.rs"));
        paths.insert(PathBuf::from("/work/repo/pkg/lib.rs"));
        paths.insert(PathBuf::from("/elsewhere/x.rs")); // no project → dropped
        let g = group_by_project(&projects, paths);
        assert_eq!(g.get("outer").map(|v| v.len()), Some(1));
        assert_eq!(g.get("inner").map(|v| v.len()), Some(1));
        assert!(!g.contains_key("nope"));
        assert_eq!(g.values().map(|v| v.len()).sum::<usize>(), 2);
    }
}
