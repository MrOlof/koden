//! P4 crash-resume (EXECUTION_PLAN §4.2-4.4). A per-pane append-only JSONL journal
//! is written on every agent-lifecycle signal (on the brain worker thread, the
//! single writer); on boot, [recover_all] folds each journal's tail into a
//! [RecoveredPane] so the UI can offer a "resume where you left off" card next to
//! each cold-rehydrated tab. Tier-2 ([resume_command]) rewrites the launch to
//! `claude --resume <id>` only when a session id was genuinely captured.
//!
//! Everything is fail-open: a broken/torn journal degrades to fewer recovery
//! cards, never a crash or a blocked startup.

pub mod cursor;
pub mod journal;
pub mod sessionkey;
pub mod tier2;

pub use cursor::{gc_resume_dir, recover_all, RecoveredPane};
pub use journal::{record_event, ResumeRecord};
pub use sessionkey::SessionKey;
pub use tier2::{resume_command, valid_session_id, ResumePlan};

/// A journal older than this many days (by mtime) is GC'd on boot (§4.4).
pub const RESUME_TTL_DAYS: i64 = 7;
