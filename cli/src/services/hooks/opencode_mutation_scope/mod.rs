#![allow(dead_code)]

mod events;
pub(crate) mod health;
mod lifecycle;
mod payload;
pub(crate) mod state;

pub(crate) use events::{format_opencode_scope_id, AttemptKey};
#[allow(unused_imports)]
pub(crate) use health::{assess_repairability, Repairability};

pub(crate) use lifecycle::run_opencode_mutation_scope_subcommand;
#[allow(unused_imports)]
pub(crate) use lifecycle::{repair_blocked, RepairOutcome};
