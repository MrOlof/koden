//! Headless REAL-kill crash sim for P4 crash-resume (BUILD-PROMPT §13.29 — the
//! crash must be a REAL process kill, not an in-process mock). Two PROCESSES share
//! a store dir:
//!   write:   open the store (same path the worker uses), journal started+working
//!            for a pane, then `std::process::abort()` — abnormal termination, no
//!            clean exit and no `exited` marker (a power-cut / SIGKILL stand-in).
//!   recover: re-open and `recover_all()` — the pane MUST come back as still-working.
//!
//! Driven by the sibling shell step:
//!   cargo run --example brain_crash_sim -- write   <dir>   # aborts (non-zero exit)
//!   cargo run --example brain_crash_sim -- recover <dir>   # prints PASS, exit 0
//! The kill happens between the two invocations, so it is a genuine cross-process
//! recovery, not a simulated one.

use std::path::PathBuf;

use koden_lib::modules::brain::resume::{record_event, recover_all, ResumeRecord, SessionKey};
use koden_lib::modules::brain::store::SqliteIndex;

fn resume_dir(base: &str) -> PathBuf {
    PathBuf::from(base).join("resume")
}

fn rec(kind: &str) -> ResumeRecord {
    ResumeRecord {
        ts: 1,
        kind: kind.into(),
        agent: Some("claude".into()),
        cwd: "/work/proj".into(),
        project: Some("p".into()),
        claude_session_id: None,
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map(String::as_str).unwrap_or("");
    let base = args.get(2).cloned().expect("usage: brain_crash_sim <write|recover> <dir>");

    match mode {
        "write" => {
            // Open the store exactly as the worker does (creates dir + migrates).
            let _idx = SqliteIndex::open(&PathBuf::from(&base).join("index.sqlite"))
                .expect("open store");
            let key = SessionKey::derive("/work/proj", "claude", None);
            let dir = resume_dir(&base);
            record_event(&dir, &key, &rec("started")).expect("journal started");
            record_event(&dir, &key, &rec("working")).expect("journal working");
            eprintln!("[crash-sim:write] journaled started+working; aborting (REAL kill)…");
            std::process::abort(); // abnormal termination: no clean exit, no 'exited'
        }
        "recover" => {
            let dir = resume_dir(&base);
            let recovered = recover_all(&dir);
            eprintln!(
                "[crash-sim:recover] recovered {} pane(s): {:?}",
                recovered.len(),
                recovered.iter().map(|p| (p.key.as_str(), p.last_kind.as_str())).collect::<Vec<_>>()
            );
            assert_eq!(recovered.len(), 1, "expected exactly one recoverable pane after the kill");
            assert_eq!(recovered[0].last_kind, "working", "pane must recover as still-working");
            assert_eq!(recovered[0].agent.as_deref(), Some("claude"));
            println!("PASS: resume journal survived a REAL process abort and recovered the working pane");
        }
        other => {
            eprintln!("unknown mode {other:?}; usage: brain_crash_sim <write|recover> <dir>");
            std::process::exit(2);
        }
    }
}
