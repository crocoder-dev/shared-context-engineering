use std::process::Command;

use serde_json::{json, Value};

const PLAN: &str = "# Plan: example

## Task stack

- [x] T01: `Create database schema` (status:done)
  - Context synchronization: synced

- [ ] T02: `Implement repository` (status:todo)

## Validation Report

**Status:** failed
**Date:** 2026-11-27

### Commands run

- `cargo test` -> exit 1
";

#[test]
fn plan_status_runs_in_unconfigured_environment() {
    let sandbox = tempfile::tempdir().unwrap();
    let home = sandbox.path().join("home");
    let work = sandbox.path().join("work");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&work).unwrap();
    let plan = work.join("plan.md");
    std::fs::write(&plan, PLAN).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_sce"))
        .args(["plan", "status", plan.to_str().unwrap(), "--format", "json"])
        .current_dir(&work)
        .env_clear()
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_STATE_HOME", home.join("state"))
        .env("XDG_DATA_HOME", home.join("data"))
        .env("XDG_CACHE_HOME", home.join("cache"))
        .env("GIT_CEILING_DIRECTORIES", sandbox.path())
        .env("NO_COLOR", "1")
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    assert_eq!(stderr, "");
    let stdout: Value = serde_json::from_slice(&output.stdout).unwrap();
    let report_sha256 = stdout["validation"]["report_sha256"].as_str().unwrap();
    assert_eq!(report_sha256.len(), 64);
    assert_eq!(
        stdout,
        json!({
            "tasks": [
                {"id": "T01", "title": "Create database schema", "status": "done", "context_sync": "synced"},
                {"id": "T02", "title": "Implement repository", "status": "todo", "context_sync": null},
            ],
            "validation": {"status": "failed", "report_sha256": report_sha256},
        })
    );
    assert_eq!(std::fs::read_dir(&home).unwrap().count(), 0);
}
