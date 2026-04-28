#![allow(dead_code)]

use std::path::PathBuf;

use anyhow::Result;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ServiceId(pub &'static str);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServiceMetadata {
    pub id: ServiceId,
    pub display_name: &'static str,
    pub description: &'static str,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LifecycleContext {
    pub repository: Option<PathBuf>,
    pub config: Option<PathBuf>,
    pub state: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LifecycleOperation {
    Setup,
    Diagnose,
    Fix,
    Preview,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SetupRequest {
    pub context: LifecycleContext,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagnoseRequest {
    pub context: LifecycleContext,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FixRequest {
    pub context: LifecycleContext,
    pub problem_kinds: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreviewRequest {
    pub context: LifecycleContext,
    pub operation: LifecycleOperation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LifecycleOutcome {
    Applied,
    Updated,
    Unchanged,
    Skipped,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiagnosticSeverity {
    Error,
    Warning,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiagnosticFixability {
    AutoFixable,
    ManualOnly,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LifecycleAction {
    pub service_id: ServiceId,
    pub operation: LifecycleOperation,
    pub target: String,
    pub description: String,
    pub outcome: LifecycleOutcome,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SetupReport {
    pub service_id: ServiceId,
    pub actions: Vec<LifecycleAction>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagnosticRecord {
    pub service_id: ServiceId,
    pub kind: String,
    pub target: String,
    pub severity: DiagnosticSeverity,
    pub fixability: DiagnosticFixability,
    pub summary: String,
    pub remediation: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagnosticReport {
    pub service_id: ServiceId,
    pub diagnostics: Vec<DiagnosticRecord>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FixReport {
    pub service_id: ServiceId,
    pub actions: Vec<LifecycleAction>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionPlan {
    pub service_id: ServiceId,
    pub operation: LifecycleOperation,
    pub actions: Vec<LifecycleAction>,
}

pub trait LifecycleService {
    fn metadata(&self) -> ServiceMetadata;
}

pub trait SetupLifecycle: LifecycleService {
    fn setup(&self, request: SetupRequest) -> Result<SetupReport>;
}

pub trait DiagnosticLifecycle: LifecycleService {
    fn diagnose(&self, request: DiagnoseRequest) -> Result<DiagnosticReport>;
}

pub trait FixLifecycle: LifecycleService {
    fn fix(&self, request: FixRequest) -> Result<FixReport>;
}

pub trait PreviewLifecycle: LifecycleService {
    fn preview(&self, request: PreviewRequest) -> Result<ActionPlan>;
}
