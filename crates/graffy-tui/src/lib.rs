//! Terminal UI (ADR-0002) — ratatui-native, macOS + Linux.
//!
//! * [`state`] — the pure fold from journal frames to renderable state
//!   (terminal-free, unit-tested against real executor runs).
//! * [`ui`] — live run view, step inspector, novice strip, journal picker.
//!
//! The TUI has no privileged access: it renders exactly the frames the
//! journal commits (live via the tap, or read back from disk). If it isn't
//! in the journal, it isn't on screen.

pub mod state;
pub mod ui;

pub use ui::{run_home, run_live, run_replay};
