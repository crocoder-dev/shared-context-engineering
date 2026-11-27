pub mod command;
pub mod parse;

use std::path::PathBuf;

use serde::Serialize;

use crate::services::output_format::OutputFormat;

pub const NAME: &str = "plan";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlanStatusRequest {
    pub path: PathBuf,
    pub format: OutputFormat,
}

/// Persisted lifecycle state of one SCE Markdown plan.
#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct PlanStatus {
    pub tasks: Vec<Task>,
    pub validation: Option<Validation>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub status: TaskStatus,
    /// `None` when a legacy task records no context synchronization field.
    pub context_sync: Option<ContextSync>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Todo,
    Done,
}

impl TaskStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Todo => "todo",
            Self::Done => "done",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextSync {
    Pending,
    Synced,
    Blocked,
}

impl ContextSync {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Synced => "synced",
            Self::Blocked => "blocked",
        }
    }
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct Validation {
    pub status: ValidationStatus,
    /// Lowercase hex SHA-256 of the report section's source bytes. It identifies
    /// the report content only and is not evidence that validation ran.
    pub report_sha256: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationStatus {
    Validated,
    Failed,
}

impl ValidationStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Validated => "validated",
            Self::Failed => "failed",
        }
    }
}
