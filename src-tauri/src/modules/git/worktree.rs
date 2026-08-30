use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

use crate::modules::git::errors::{GitError, Result};
use crate::modules::git::parser::{parse_ref_list, parse_worktree_list};
use crate::modules::git::process::{
    ensure_git_available, ensure_success, git_stdout_line_opt, run_git,
};
use crate::modules::git::types::{
    GitBranches, GitLinkOutcome, GitLinkResult, GitWorktree, DEFAULT_TIMEOUT_SECS,
    WORKTREE_TIMEOUT_SECS,
};
use crate::modules::git::utils::{
    authorized_repo_root, canonical_dir, display_path, is_safe_pathspec, resolve_within_repo,
    ResolvedGitDirectory,
};
use crate::modules::workspace::{resolve_path, WorkspaceEnv, WorkspaceRegistry};

const KODEN_DIR: &str = ".koden";
const MAX_REF_LEN: usize = 255;

pub fn branches(
    registry: &WorkspaceRegistry,
    repo_root: &str,
    workspace: &WorkspaceEnv,
) -> Result<GitBranches> {
    let repo_root = authorized_repo_root(registry, repo_root, workspace)?;
    ensure_git_available(&repo_root.workspace)?;
    let current = git_stdout_line_opt(
        &repo_root.workspace,
        &repo_root.git_path,
        ["symbolic-ref", "--short", "-q", "HEAD"],
    )?;
    let output = run_git(
        &repo_root.workspace,
        Some(&repo_root.git_path),
        [
            "for-each-ref",
            "--format=%(refname)",
            "refs/heads",
            "refs/remotes",
        ],
        DEFAULT_TIMEOUT_SECS,
    )?;
    ensure_success(&output, "git for-each-ref failed")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let refs = parse_ref_list(stdout.lines());
    Ok(GitBranches {
        current,
        local: refs.local,
        remote: refs.remote,
    })
}

pub fn list(
    registry: &WorkspaceRegistry,
    repo_root: &str,
    workspace: &WorkspaceEnv,
) -> Result<Vec<GitWorktree>> {
    let repo_root = authorized_repo_root(registry, repo_root, workspace)?;
    ensure_git_available(&repo_root.workspace)?;
    list_inner(&repo_root)
}

fn list_inner(repo_root: &ResolvedGitDirectory) -> Result<Vec<GitWorktree>> {
    let output = run_git(
        &repo_root.workspace,
        Some(&repo_root.git_path),
        ["worktree", "list", "--porcelain"],
        DEFAULT_TIMEOUT_SECS,
    )?;
    ensure_success(&output, "git worktree list failed")?;
    Ok(parse_worktree_list(&String::from_utf8_lossy(&output.stdout)))
}

pub fn add(
    registry: &WorkspaceRegistry,
    repo_root: &str,
    path: &str,
    new_branch: Option<&str>,
    base: &str,
    workspace: &WorkspaceEnv,
) -> Result<GitWorktree> {
    let repo_root = authorized_repo_root(registry, repo_root, workspace)?;
    ensure_git_available(&repo_root.workspace)?;
    if let Some(name) = new_branch {
        validate_ref_name(&repo_root, name)?;
    }
    validate_ref_name(&repo_root, base)?;

    let target = WorktreeTarget::resolve(&repo_root, path)?;
    if target.local.is_file() {
        return Err(GitError::InvalidPath(path.to_string()));
    }
    if target.under_koden_dir(&repo_root.local_path) {
        ensure_koden_gitignore(&repo_root.local_path)?;
    }
    if let Some(parent) = target.local.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut args: Vec<OsString> = vec!["worktree".into(), "add".into()];
    if let Some(name) = new_branch {
        args.push("-b".into());
        args.push(name.into());
    }
    args.push("--".into());
    args.push(target.git_path.clone().into());
    args.push(base.into());
    let output = run_git(
        &repo_root.workspace,
        Some(&repo_root.git_path),
        args,
        WORKTREE_TIMEOUT_SECS,
    )?;
    ensure_success(&output, "git worktree add failed")?;

    let created = canonical_dir(registry, &target.git_path, &repo_root.workspace)?;
    let _ = registry.authorize(&created.local_path);

    let entries = list_inner(&repo_root)?;
    let mut found = entries
        .into_iter()
        .find(|wt| !wt.is_main && entry_matches(registry, wt, &created))
        .ok_or_else(|| {
            GitError::command(
                "git worktree add",
                "worktree created but not listed afterwards",
            )
        })?;
    found.path = created.git_path;
    Ok(found)
}

pub fn remove(
    registry: &WorkspaceRegistry,
    repo_root: &str,
    path: &str,
    force: bool,
    workspace: &WorkspaceEnv,
) -> Result<()> {
    let repo_root = authorized_repo_root(registry, repo_root, workspace)?;
    ensure_git_available(&repo_root.workspace)?;
    let entries = list_inner(&repo_root)?;
    let requested = resolve_path(path, &repo_root.workspace);
    let requested_canonical = std::fs::canonicalize(&requested).ok();
    let target = entries
        .iter()
        .find(|wt| {
            let listed = resolve_path(&wt.path, &repo_root.workspace);
            match (&requested_canonical, std::fs::canonicalize(&listed).ok()) {
                (Some(a), Some(b)) => a == &b,
                _ => same_path_text(&wt.path, path),
            }
        })
        .ok_or_else(|| GitError::command("git worktree remove", "path is not a linked worktree"))?;
    if target.is_main {
        return Err(GitError::command(
            "git worktree remove",
            "refusing to remove the main worktree",
        ));
    }

    // Only ever delete a directory the registry vouches for; a registration
    // whose checkout is already gone is dropped with prune, which touches
    // nothing on disk.
    if let Some(canonical) = requested_canonical.as_ref() {
        if !registry.is_authorized(canonical) {
            return Err(GitError::PathOutsideWorkspace(canonical.clone()));
        }
        detach_links(canonical);
    }

    let mut args: Vec<OsString> = vec!["worktree".into(), "remove".into()];
    if force {
        args.push("--force".into());
    }
    args.push("--".into());
    args.push(target.path.clone().into());
    let output = run_git(
        &repo_root.workspace,
        Some(&repo_root.git_path),
        args,
        WORKTREE_TIMEOUT_SECS,
    )?;
    if output.exit_code != Some(0) && !output.timed_out && requested_canonical.is_none() {
        let prune = run_git(
            &repo_root.workspace,
            Some(&repo_root.git_path),
            ["worktree", "prune"],
            DEFAULT_TIMEOUT_SECS,
        )?;
        return ensure_success(&prune, "git worktree prune failed");
    }
    ensure_success(&output, "git worktree remove failed")
}

pub fn link_paths(
    registry: &WorkspaceRegistry,
    source_root: &str,
    target_root: &str,
    rel_paths: &[String],
    workspace: &WorkspaceEnv,
) -> Result<Vec<GitLinkResult>> {
    if !matches!(workspace, WorkspaceEnv::Local) {
        return Err(GitError::command(
            "link paths",
            "linking worktree folders is only supported in the local workspace",
        ));
    }
    let source = authorized_repo_root(registry, source_root, workspace)?;
    let target = authorized_repo_root(registry, target_root, workspace)?;
    Ok(rel_paths
        .iter()
        .map(|rel| link_one(&source.local_path, &target.local_path, rel))
        .collect())
}

fn link_one(source_root: &Path, target_root: &Path, rel: &str) -> GitLinkResult {
    let rel = rel.trim().trim_matches(|c| c == '/' || c == '\\');
    let result = |outcome: GitLinkOutcome, detail: Option<String>| GitLinkResult {
        path: rel.to_string(),
        outcome,
        detail,
    };
    if !is_safe_pathspec(rel) || rel.split(['/', '\\']).any(|seg| seg == "..") {
        return result(GitLinkOutcome::Failed, Some("invalid path".into()));
    }
    // Probe the target before canonicalizing: an existing link there resolves
    // to the source tree and would read as "outside the target".
    if std::fs::symlink_metadata(target_root.join(rel)).is_ok() {
        return result(
            GitLinkOutcome::Skipped,
            Some("already exists in target".into()),
        );
    }
    let src = match resolve_within_repo(source_root, rel) {
        Ok(p) => p,
        Err(e) => return result(GitLinkOutcome::Failed, Some(e.to_string())),
    };
    let src_meta = match std::fs::symlink_metadata(&src) {
        Ok(m) => m,
        Err(_) => return result(GitLinkOutcome::Skipped, Some("missing in source".into())),
    };
    let dst = match resolve_within_repo(target_root, rel) {
        Ok(p) => p,
        Err(e) => return result(GitLinkOutcome::Failed, Some(e.to_string())),
    };
    match create_link(&src, &dst, src_meta.is_dir()) {
        Ok(()) => result(GitLinkOutcome::Linked, None),
        Err(e) => result(GitLinkOutcome::Failed, Some(e.to_string())),
    }
}

#[cfg(windows)]
fn create_link(src: &Path, dst: &Path, is_dir: bool) -> std::io::Result<()> {
    if !is_dir {
        return std::os::windows::fs::symlink_file(src, dst);
    }
    // Directory symlinks need SeCreateSymbolicLinkPrivilege; a junction does
    // not, and cmd's mklink is the only privilege-free way to make one without
    // a new dependency.
    let to_win = |p: &Path| display_path(p).replace('/', "\\");
    let (src_win, dst_win) = (to_win(src), to_win(dst));
    if [&src_win, &dst_win]
        .iter()
        .any(|p| p.contains('%') || p.contains('"'))
    {
        return Err(std::io::Error::other(
            "path contains characters cmd cannot pass safely",
        ));
    }
    let mut cmd = std::process::Command::new("cmd");
    cmd.args(["/D", "/C", "mklink", "/J"])
        .arg(&dst_win)
        .arg(&src_win)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    crate::modules::proc::hide_console(&mut cmd);
    let out = cmd.output()?;
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    Err(std::io::Error::other(if stderr.is_empty() {
        stdout
    } else {
        stderr
    }))
}

#[cfg(unix)]
fn create_link(src: &Path, dst: &Path, _is_dir: bool) -> std::io::Result<()> {
    std::os::unix::fs::symlink(src, dst)
}

/// Unlink directory reparse points / symlinks inside a checkout before git
/// deletes it, so a junctioned `node_modules` can never drag the main
/// checkout's copy down with the worktree. Best effort; `.git` is skipped.
fn detach_links(root: &Path) {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.file_name().is_some_and(|n| n == ".git") {
                continue;
            }
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.file_type().is_symlink() || is_junction(&meta) {
                let _ = unlink_dir_link(&path, &meta);
                continue;
            }
            if meta.is_dir() {
                stack.push(path);
            }
        }
    }
}

#[cfg(windows)]
fn is_junction(meta: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_junction(_meta: &std::fs::Metadata) -> bool {
    false
}

fn unlink_dir_link(path: &Path, meta: &std::fs::Metadata) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        if meta.is_dir() || is_junction(meta) {
            return std::fs::remove_dir(path);
        }
    }
    let _ = meta;
    std::fs::remove_file(path)
}

fn validate_ref_name(repo_root: &ResolvedGitDirectory, name: &str) -> Result<()> {
    if !is_plausible_ref_name(name) {
        return Err(GitError::command("invalid branch name", name));
    }
    let output = run_git(
        &repo_root.workspace,
        Some(&repo_root.git_path),
        ["check-ref-format", "--branch", name],
        DEFAULT_TIMEOUT_SECS,
    )?;
    if output.timed_out {
        return Err(GitError::TimedOut("git check-ref-format"));
    }
    if output.exit_code != Some(0) {
        return Err(GitError::command("invalid branch name", name));
    }
    Ok(())
}

// Cheap prefilter before git's own check: keeps option-looking and control
// laden strings from ever reaching argv.
pub(crate) fn is_plausible_ref_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_REF_LEN
        && !name.starts_with('-')
        && !name.chars().any(|c| c.is_control() || c.is_whitespace())
}

struct WorktreeTarget {
    local: PathBuf,
    git_path: String,
}

impl WorktreeTarget {
    fn resolve(repo_root: &ResolvedGitDirectory, path: &str) -> Result<Self> {
        if path.is_empty()
            || path.chars().any(|c| c.is_control())
            || path.split(['/', '\\']).any(|seg| seg == "..")
        {
            return Err(GitError::InvalidPath(path.to_string()));
        }
        let is_absolute = Path::new(path).is_absolute() || path.starts_with('/');
        let candidate = if is_absolute {
            resolve_path(path, &repo_root.workspace)
        } else {
            repo_root.local_path.join(path)
        };
        let local = contained_target(&repo_root.local_path, &candidate)?;
        let git_path = if repo_root.workspace.is_wsl() {
            if is_absolute {
                path.replace('\\', "/")
            } else {
                format!(
                    "{}/{}",
                    repo_root.git_path.trim_end_matches('/'),
                    path.replace('\\', "/")
                )
            }
        } else {
            display_path(&local)
        };
        Ok(Self { local, git_path })
    }

    fn under_koden_dir(&self, repo_root: &Path) -> bool {
        first_component_is(repo_root, &self.local, KODEN_DIR)
    }
}

fn first_component_is(repo_root: &Path, path: &Path, name: &str) -> bool {
    path.strip_prefix(repo_root)
        .ok()
        .and_then(|rel| rel.components().next())
        .is_some_and(|c| matches!(c, Component::Normal(n) if n == name))
}

/// Canonicalize through the nearest existing ancestor so a not-yet-created
/// target still resolves symlinks in its parents, then require it to sit
/// strictly inside the repo root and outside `.git`.
fn contained_target(repo_root: &Path, candidate: &Path) -> Result<PathBuf> {
    if candidate
        .components()
        .any(|c| matches!(c, Component::ParentDir))
    {
        return Err(GitError::InvalidPath(candidate.display().to_string()));
    }
    let mut existing = candidate.to_path_buf();
    let mut tail: Vec<OsString> = Vec::new();
    while !existing.exists() {
        let name = existing
            .file_name()
            .ok_or_else(|| GitError::InvalidPath(candidate.display().to_string()))?
            .to_os_string();
        tail.push(name);
        existing = existing
            .parent()
            .ok_or_else(|| GitError::InvalidPath(candidate.display().to_string()))?
            .to_path_buf();
    }
    let mut canonical = std::fs::canonicalize(&existing)?;
    for name in tail.iter().rev() {
        canonical.push(name);
    }
    if canonical == repo_root || !canonical.starts_with(repo_root) {
        return Err(GitError::PathOutsideWorkspace(canonical));
    }
    if first_component_is(repo_root, &canonical, ".git") {
        return Err(GitError::InvalidPath(canonical.display().to_string()));
    }
    Ok(canonical)
}

fn ensure_koden_gitignore(repo_root: &Path) -> Result<()> {
    let dir = repo_root.join(KODEN_DIR);
    std::fs::create_dir_all(&dir)?;
    let ignore = dir.join(".gitignore");
    if !ignore.exists() {
        std::fs::write(&ignore, "*\n")?;
    }
    Ok(())
}

fn entry_matches(
    registry: &WorkspaceRegistry,
    entry: &GitWorktree,
    created: &ResolvedGitDirectory,
) -> bool {
    match canonical_dir(registry, &entry.path, &created.workspace) {
        Ok(listed) => listed.local_path == created.local_path,
        Err(_) => same_path_text(&entry.path, &created.git_path),
    }
}

fn same_path_text(a: &str, b: &str) -> bool {
    let norm = |s: &str| {
        let s = s.replace('\\', "/");
        let s = s.trim_end_matches('/').to_string();
        if cfg!(windows) {
            s.to_ascii_lowercase()
        } else {
            s
        }
    };
    norm(a) == norm(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn git(root: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        assert!(ok, "git {args:?} failed");
    }

    fn git_stdout(root: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .expect("git runs");
        assert!(out.status.success(), "git {args:?} failed");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    struct Repo {
        _dir: tempfile::TempDir,
        registry: WorkspaceRegistry,
        root: String,
        local: PathBuf,
    }

    fn init_repo() -> Repo {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        git(root, &["init", "-q"]);
        git(root, &["symbolic-ref", "HEAD", "refs/heads/main"]);
        git(root, &["config", "user.email", "test@koden.local"]);
        git(root, &["config", "user.name", "Koden Test"]);
        git(root, &["config", "commit.gpgsign", "false"]);
        std::fs::write(root.join("a.txt"), b"x").unwrap();
        git(root, &["add", "."]);
        git(root, &["commit", "-q", "-m", "init"]);
        let registry = WorkspaceRegistry::default();
        let local = registry.authorize(root).unwrap();
        Repo {
            root: display_path(&local),
            local,
            registry,
            _dir: dir,
        }
    }

    const LOCAL: WorkspaceEnv = WorkspaceEnv::Local;

    #[test]
    fn branches_lists_current_local_and_remote_without_remote_head() {
        let repo = init_repo();
        let root = &repo.local;
        git(root, &["branch", "feature"]);
        git(root, &["remote", "add", "origin", &repo.root]);
        git(root, &["fetch", "-q", "origin"]);
        git(
            root,
            &[
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/main",
            ],
        );

        let b = branches(&repo.registry, &repo.root, &LOCAL).unwrap();
        assert_eq!(b.current.as_deref(), Some("main"));
        assert_eq!(b.local, vec!["feature", "main"]);
        assert_eq!(b.remote, vec!["origin/feature", "origin/main"]);
    }

    #[test]
    fn branches_current_is_none_when_detached() {
        let repo = init_repo();
        git(&repo.local, &["checkout", "-q", "--detach"]);
        let b = branches(&repo.registry, &repo.root, &LOCAL).unwrap();
        assert!(b.current.is_none());
        assert_eq!(b.local, vec!["main"]);
    }

    #[test]
    fn add_with_new_branch_creates_checkout_gitignore_and_authorizes() {
        let repo = init_repo();
        let wt = add(
            &repo.registry,
            &repo.root,
            ".koden/worktrees/feat-x",
            Some("feat/x"),
            "main",
            &LOCAL,
        )
        .unwrap();
        assert_eq!(wt.branch.as_deref(), Some("feat/x"));
        assert!(!wt.is_main);
        assert!(wt.path.ends_with(".koden/worktrees/feat-x"), "{}", wt.path);
        assert!(!wt.head.is_empty());

        let checkout = repo.local.join(".koden").join("worktrees").join("feat-x");
        assert!(checkout.join("a.txt").is_file());
        assert!(repo.registry.is_authorized(&checkout));

        let ignore = std::fs::read_to_string(repo.local.join(".koden").join(".gitignore")).unwrap();
        assert_eq!(ignore, "*\n");
        assert_eq!(git_stdout(&repo.local, &["status", "--porcelain"]), "");

        let listed = list(&repo.registry, &repo.root, &LOCAL).unwrap();
        assert_eq!(listed.len(), 2);
        assert!(listed[0].is_main);
        assert_eq!(listed[1].branch.as_deref(), Some("feat/x"));
    }

    #[test]
    fn add_existing_branch_checks_it_out_and_leaves_koden_dir_alone() {
        let repo = init_repo();
        git(&repo.local, &["branch", "existing"]);
        let wt = add(
            &repo.registry,
            &repo.root,
            "wt-existing",
            None,
            "existing",
            &LOCAL,
        )
        .unwrap();
        assert_eq!(wt.branch.as_deref(), Some("existing"));
        assert!(repo.local.join("wt-existing").join("a.txt").is_file());
        assert!(!repo.local.join(KODEN_DIR).exists());
    }

    #[test]
    fn add_rejects_paths_outside_the_repo() {
        let repo = init_repo();
        let err = add(
            &repo.registry,
            &repo.root,
            "../escape",
            Some("x"),
            "main",
            &LOCAL,
        )
        .unwrap_err();
        assert!(matches!(err, GitError::InvalidPath(_)), "{err}");

        let outside = tempfile::tempdir().unwrap();
        let abs = display_path(&outside.path().join("wt"));
        let err = add(&repo.registry, &repo.root, &abs, Some("y"), "main", &LOCAL).unwrap_err();
        assert!(matches!(err, GitError::PathOutsideWorkspace(_)), "{err}");

        let err = add(&repo.registry, &repo.root, ".git/wt", Some("z"), "main", &LOCAL).unwrap_err();
        assert!(matches!(err, GitError::InvalidPath(_)), "{err}");

        let err = add(&repo.registry, &repo.root, &repo.root, Some("w"), "main", &LOCAL).unwrap_err();
        assert!(matches!(err, GitError::PathOutsideWorkspace(_)), "{err}");

        assert_eq!(list(&repo.registry, &repo.root, &LOCAL).unwrap().len(), 1);
    }

    #[test]
    fn add_rejects_invalid_branch_names_before_touching_disk() {
        let repo = init_repo();
        for bad in ["bad name", "-flag", "a..b", "x/", "", "feat\nx"] {
            let err = add(
                &repo.registry,
                &repo.root,
                ".koden/worktrees/z",
                Some(bad),
                "main",
                &LOCAL,
            )
            .unwrap_err();
            assert!(err.to_string().contains("invalid branch name"), "{bad:?}: {err}");
        }
        let err = add(
            &repo.registry,
            &repo.root,
            ".koden/worktrees/z",
            Some("ok"),
            "--upload-pack=evil",
            &LOCAL,
        )
        .unwrap_err();
        assert!(err.to_string().contains("invalid branch name"), "{err}");
        assert!(!repo.local.join(KODEN_DIR).exists());
    }

    #[test]
    fn remove_refuses_main_worktree() {
        let repo = init_repo();
        let err = remove(&repo.registry, &repo.root, &repo.root, false, &LOCAL).unwrap_err();
        assert!(err.to_string().contains("main worktree"), "{err}");
        assert!(repo.local.join("a.txt").is_file());
    }

    #[test]
    fn remove_rejects_unlisted_paths() {
        let repo = init_repo();
        let stray = repo.local.join("stray");
        std::fs::create_dir_all(&stray).unwrap();
        let err = remove(
            &repo.registry,
            &repo.root,
            &display_path(&stray),
            true,
            &LOCAL,
        )
        .unwrap_err();
        assert!(err.to_string().contains("not a linked worktree"), "{err}");
        assert!(stray.is_dir());
    }

    #[test]
    fn remove_deletes_linked_worktree_and_keeps_branch() {
        let repo = init_repo();
        let wt = add(
            &repo.registry,
            &repo.root,
            ".koden/worktrees/gone",
            Some("feat/gone"),
            "main",
            &LOCAL,
        )
        .unwrap();
        remove(&repo.registry, &repo.root, &wt.path, false, &LOCAL).unwrap();
        assert!(!repo.local.join(".koden").join("worktrees").join("gone").exists());
        assert_eq!(list(&repo.registry, &repo.root, &LOCAL).unwrap().len(), 1);
        let b = branches(&repo.registry, &repo.root, &LOCAL).unwrap();
        assert!(b.local.iter().any(|n| n == "feat/gone"));
    }

    #[test]
    fn remove_prunes_a_registration_whose_checkout_is_already_gone() {
        let repo = init_repo();
        let wt = add(
            &repo.registry,
            &repo.root,
            ".koden/worktrees/vanished",
            Some("feat/vanished"),
            "main",
            &LOCAL,
        )
        .unwrap();
        std::fs::remove_dir_all(repo.local.join(".koden").join("worktrees").join("vanished")).unwrap();
        remove(&repo.registry, &repo.root, &wt.path, false, &LOCAL).unwrap();
        assert_eq!(list(&repo.registry, &repo.root, &LOCAL).unwrap().len(), 1);
    }

    #[test]
    fn link_paths_links_existing_skips_missing_and_survives_removal() {
        let repo = init_repo();
        let src_modules = repo.local.join("node_modules").join("pkg");
        std::fs::create_dir_all(&src_modules).unwrap();
        std::fs::write(src_modules.join("index.js"), b"module.exports = 1;\n").unwrap();
        std::fs::write(repo.local.join(".gitignore"), b"node_modules\n").unwrap();
        git(&repo.local, &["add", ".gitignore"]);
        git(&repo.local, &["commit", "-q", "-m", "ignore modules"]);

        let wt = add(
            &repo.registry,
            &repo.root,
            ".koden/worktrees/linked",
            Some("feat/linked"),
            "main",
            &LOCAL,
        )
        .unwrap();
        let results = link_paths(
            &repo.registry,
            &repo.root,
            &wt.path,
            &["node_modules".into(), ".venv".into(), "../outside".into()],
            &LOCAL,
        )
        .unwrap();
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].outcome, GitLinkOutcome::Linked, "{:?}", results[0]);
        assert_eq!(results[1].outcome, GitLinkOutcome::Skipped, "{:?}", results[1]);
        assert_eq!(results[2].outcome, GitLinkOutcome::Failed, "{:?}", results[2]);

        let checkout = repo.local.join(".koden").join("worktrees").join("linked");
        assert!(checkout.join("node_modules").join("pkg").join("index.js").is_file());

        let again = link_paths(
            &repo.registry,
            &repo.root,
            &wt.path,
            &["node_modules".into()],
            &LOCAL,
        )
        .unwrap();
        assert_eq!(again[0].outcome, GitLinkOutcome::Skipped, "{:?}", again[0]);

        remove(&repo.registry, &repo.root, &wt.path, true, &LOCAL).unwrap();
        assert!(!checkout.exists());
        assert!(
            src_modules.join("index.js").is_file(),
            "removing the worktree must not follow the link into the source"
        );
    }

    #[test]
    fn link_paths_is_local_only() {
        let repo = init_repo();
        let err = link_paths(
            &repo.registry,
            &repo.root,
            &repo.root,
            &["node_modules".into()],
            &WorkspaceEnv::Wsl {
                distro: "Ubuntu".into(),
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("local workspace"), "{err}");
    }

    #[test]
    fn unauthorized_repo_root_is_rejected_everywhere() {
        let repo = init_repo();
        let registry = WorkspaceRegistry::default();
        assert!(matches!(
            branches(&registry, &repo.root, &LOCAL),
            Err(GitError::PathOutsideWorkspace(_))
        ));
        assert!(matches!(
            list(&registry, &repo.root, &LOCAL),
            Err(GitError::PathOutsideWorkspace(_))
        ));
        assert!(matches!(
            add(&registry, &repo.root, "wt", Some("b"), "main", &LOCAL),
            Err(GitError::PathOutsideWorkspace(_))
        ));
        assert!(matches!(
            remove(&registry, &repo.root, "wt", false, &LOCAL),
            Err(GitError::PathOutsideWorkspace(_))
        ));
    }

    #[test]
    fn plausible_ref_name_prefilter() {
        assert!(is_plausible_ref_name("feat/x"));
        assert!(is_plausible_ref_name("origin/main"));
        assert!(!is_plausible_ref_name(""));
        assert!(!is_plausible_ref_name("-b"));
        assert!(!is_plausible_ref_name("a b"));
        assert!(!is_plausible_ref_name("a\tb"));
        assert!(!is_plausible_ref_name(&"x".repeat(MAX_REF_LEN + 1)));
    }

    #[test]
    fn same_path_text_normalizes_separators_and_trailing_slash() {
        assert!(same_path_text("C:/a/b/", "C:\\a\\b"));
        assert!(!same_path_text("/a/b", "/a/c"));
    }
}
