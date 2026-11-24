use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::Result;
use serde_json::Value;
use tokio::net::TcpListener;

use crate::services::error::CliError;
use crate::services::managed_telemetry::trace_export::tests::{
    capture_requests_until, loopback_telemetry_of_kind, trace_spans, DecodedSpan,
};
use crate::services::managed_telemetry::trace_export::{
    record_codex_hook_command, record_codex_hook_payload, CodexHookCommand, ManagedSpanKind,
    ManagedTraceTelemetry, SHUTDOWN_TIMEOUT,
};
use crate::services::observability::traits::{NoopLogger, NoopTelemetry, Telemetry};

use super::{run_codex_mutation_scope_from_payload_with_seams, CodexBashPolicyDecision};

const PRE_APPLY_PATCH: &str =
    include_str!("fixtures/probe01-apply-patch-and-shell-success.apply_patch.pre_tool_use.json");
const POST_APPLY_PATCH: &str =
    include_str!("fixtures/probe01-apply-patch-and-shell-success.apply_patch.post_tool_use.json");

struct Workspace {
    _dir: tempfile::TempDir,
    project: PathBuf,
    git_dir: PathBuf,
}

fn workspace() -> Workspace {
    let dir = tempfile::tempdir().expect("temp dir");
    let project = dir.path().join("project");
    let git_dir = dir.path().join("git");
    std::fs::create_dir_all(&project).expect("project dir");
    std::fs::create_dir_all(&git_dir).expect("git dir");
    Workspace {
        _dir: dir,
        project,
        git_dir,
    }
}

fn payload(fixture: &str, project: &Path, overrides: &[(&str, &str)]) -> String {
    let mut value: Value = serde_json::from_str(fixture).expect("fixture JSON");
    value["cwd"] = Value::String(project.to_string_lossy().into_owned());
    for (key, field) in overrides {
        value[*key] = Value::String((*field).to_string());
    }
    value.to_string()
}

async fn run_hook(
    stdin: &str,
    git_dir: &Path,
    seam_payloads: &Mutex<Vec<String>>,
) -> Result<String, CliError> {
    record_codex_hook_command(CodexHookCommand::CodexMutationScope);
    record_codex_hook_payload(stdin);
    let resolve_git_dir = async |_: &str| -> Result<PathBuf> { Ok(git_dir.to_path_buf()) };
    let seam = async |_: &Path, payload: &str, _: Option<&NoopLogger>| -> Result<String> {
        seam_payloads.lock().unwrap().push(payload.to_string());
        Ok(String::new())
    };
    let bash_policy = |_: &Path, _: &str| -> Result<CodexBashPolicyDecision> {
        panic!("apply_patch fixtures never evaluate bash policy")
    };
    run_codex_mutation_scope_from_payload_with_seams(
        stdin,
        None::<&NoopLogger>,
        &resolve_git_dir,
        &seam,
        &bash_policy,
    )
    .await
    .map_err(CliError::runtime)
}

async fn run_with<T: Telemetry>(
    telemetry: &T,
    stdin: &str,
    git_dir: &Path,
    seam_payloads: &Mutex<Vec<String>>,
) -> String {
    telemetry
        .with_default_subscriber(&mut || run_hook(stdin, git_dir, seam_payloads))
        .await
        .expect("hook result")
}

async fn exported_spans_for(stdin: &str, workspace: &Workspace) -> Vec<DecodedSpan> {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let address = listener.local_addr().expect("addr");
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let receiver = tokio::spawn(capture_requests_until(listener, stopped));
    let telemetry =
        loopback_telemetry_of_kind(address, "bao-token-value", ManagedSpanKind::CodexHook);
    let seam_payloads = Mutex::new(Vec::new());
    run_with(&telemetry, stdin, &workspace.git_dir, &seam_payloads).await;
    telemetry.shutdown().await;
    stop.send(()).expect("stop receiver");
    trace_spans(&receiver.await.expect("receiver join"))
}

fn attribute<'a>(span: &'a DecodedSpan, key: &str) -> Option<&'a str> {
    span.attributes
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.as_str())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn codex_mutation_scope_apply_patch_exports_one_hook_root_with_mutation_scope_children() {
    let workspace = workspace();
    for (fixture, event) in [
        (PRE_APPLY_PATCH, "PreToolUse"),
        (POST_APPLY_PATCH, "PostToolUse"),
    ] {
        let stdin = payload(fixture, &workspace.project, &[]);
        let spans = exported_spans_for(&stdin, &workspace).await;

        let hooks = spans
            .iter()
            .filter(|span| span.name == "sce.hook")
            .collect::<Vec<_>>();
        assert_eq!(hooks.len(), 1, "{event}: {spans:?}");
        let hook = hooks[0];
        assert!(
            hook.parent_span_id.is_empty(),
            "{event}: hook must be a root"
        );
        for (key, value) in [
            ("sce.agent", "codex"),
            ("sce.hook.command", "codex-mutation-scope"),
            ("sce.hook.event", event),
            ("sce.hook.tool", "apply_patch"),
            (
                "sce.codex.session_id",
                "01a07c1e-e08e-7172-8032-cb9d62af21d9",
            ),
            ("sce.codex.turn_id", "01a07c1e-e0cc-75f1-a566-e790c06cb033"),
            (
                "sce.codex.tool_use_id",
                "exec-1af8025d-ad98-415e-ab60-9eff31a15b73",
            ),
            ("sce.codex.model", "gpt-5.6-sol"),
            ("sce.outcome", "success"),
        ] {
            assert_eq!(attribute(hook, key), Some(value), "{event}: {key}");
        }
        assert_eq!(attribute(hook, "sce.codex.agent_id"), None, "{event}");

        let children = spans
            .iter()
            .filter(|span| span.name != "sce.hook")
            .collect::<Vec<_>>();
        assert!(
            !children.is_empty(),
            "{event}: expected a mutation-scope child"
        );
        for child in children {
            assert_eq!(child.name, "sce.mutation_scope", "{event}");
            assert_eq!(child.trace_id, hook.trace_id, "{event}");
            assert_eq!(child.parent_span_id, hook.span_id, "{event}");
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn codex_mutation_scope_untracked_tool_exports_only_the_hook_span() {
    let workspace = workspace();
    let stdin = payload(
        POST_APPLY_PATCH,
        &workspace.project,
        &[
            ("tool_name", "web_search"),
            ("agent_id", "019a-child"),
            ("agent_type", "explorer"),
        ],
    );
    let spans = exported_spans_for(&stdin, &workspace).await;

    assert_eq!(spans.len(), 1, "{spans:?}");
    assert_eq!(spans[0].name, "sce.hook");
    assert_eq!(attribute(&spans[0], "sce.hook.tool"), Some("web_search"));
    assert_eq!(
        attribute(&spans[0], "sce.codex.agent_id"),
        Some("019a-child")
    );
    assert_eq!(
        attribute(&spans[0], "sce.codex.agent_type"),
        Some("explorer")
    );
}

struct Observed {
    outputs: Vec<String>,
    seam_payloads: Vec<String>,
    state_files: Vec<(String, Vec<u8>)>,
}

fn state_files(git_dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut files = Vec::new();
    let mut pending = vec![git_dir.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).expect("read state dir") {
            let path = entry.expect("state entry").path();
            if path.is_dir() {
                pending.push(path);
            } else {
                let relative = path
                    .strip_prefix(git_dir)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                files.push((relative, std::fs::read(&path).expect("read state file")));
            }
        }
    }
    files.sort();
    files
}

async fn observe<T: Telemetry>(telemetry: &T) -> Observed {
    let workspace = workspace();
    let seam_payloads = Mutex::new(Vec::new());
    let mut outputs = Vec::new();
    for fixture in [PRE_APPLY_PATCH, POST_APPLY_PATCH] {
        let stdin = payload(fixture, &workspace.project, &[]);
        outputs.push(run_with(telemetry, &stdin, &workspace.git_dir, &seam_payloads).await);
    }
    Observed {
        outputs,
        seam_payloads: seam_payloads.into_inner().unwrap(),
        state_files: state_files(&workspace.git_dir),
    }
}

fn normalize_paths(observed: &Observed) -> Vec<String> {
    observed
        .seam_payloads
        .iter()
        .map(|payload| {
            let mut value: Value =
                serde_json::from_str(payload).unwrap_or(Value::String(payload.clone()));
            if let Some(object) = value.as_object_mut() {
                object.remove("cwd");
            }
            value.to_string()
        })
        .collect()
}

async fn assert_failing_exporter_matches_noop(failing: ManagedTraceTelemetry) {
    let baseline = observe(&NoopTelemetry).await;
    let started = Instant::now();
    let observed = observe(&failing).await;
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "hooks waited on export"
    );
    let shutdown_started = Instant::now();
    failing.shutdown().await;
    assert!(shutdown_started.elapsed() < SHUTDOWN_TIMEOUT + Duration::from_millis(500));

    assert_eq!(observed.outputs, baseline.outputs);
    assert_eq!(normalize_paths(&observed), normalize_paths(&baseline));
    assert_eq!(
        observed
            .state_files
            .iter()
            .map(|(path, _)| path)
            .collect::<Vec<_>>(),
        baseline
            .state_files
            .iter()
            .map(|(path, _)| path)
            .collect::<Vec<_>>()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn codex_mutation_scope_refused_receiver_matches_telemetry_disabled_run() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let address = listener.local_addr().expect("addr");
    drop(listener);
    assert_failing_exporter_matches_noop(loopback_telemetry_of_kind(
        address,
        "bao-token-value",
        ManagedSpanKind::CodexHook,
    ))
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn codex_mutation_scope_stalled_receiver_matches_telemetry_disabled_run() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let address = listener.local_addr().expect("addr");
    let holder = tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((stream, _)) = listener.accept().await {
            held.push(stream);
        }
    });
    assert_failing_exporter_matches_noop(loopback_telemetry_of_kind(
        address,
        "bao-token-value",
        ManagedSpanKind::CodexHook,
    ))
    .await;
    holder.abort();
}
