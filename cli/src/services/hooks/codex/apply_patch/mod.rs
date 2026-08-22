//! Codex `apply_patch` before/after attribution.
//!
//! `pre` captures the `PreToolUse(apply_patch)` before-state snapshot. `post`
//! finalizes it into an observed diff on `PostToolUse(apply_patch)`.
//! Persisting a non-empty diff as `diff_traces`/conversation evidence is a
//! later task's responsibility.

mod post;
mod pre;

pub(super) use post::handle as handle_post;
pub(super) use pre::handle;
