//! Codex `apply_patch` before/after attribution.
//!
//! `pre` captures the `PreToolUse(apply_patch)` before-state snapshot (this
//! task). `post` (a later task) finalizes it into an observed diff on
//! `PostToolUse(apply_patch)`.

mod pre;

pub(super) use pre::handle;
