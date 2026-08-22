//! Codex `apply_patch` before/after attribution.
//!
//! `pre` captures the `PreToolUse(apply_patch)` before-state snapshot. `post`
//! finalizes it into an observed diff on `PostToolUse(apply_patch)` and, for a
//! non-empty diff, `persist` writes it as `diff_traces`/conversation evidence.

mod persist;
mod post;
mod pre;

pub(super) use post::handle as handle_post;
pub(super) use pre::handle;
