use std::ffi::OsStr;
use std::fs;

use serde_json::Value;

mod support;

use support::{render_command_result, BinaryIntegrationHarness, TestResult};

type ConfigIntegrationHarness = BinaryIntegrationHarness;

#[test]
fn config_show_flags_override_env_and_config_values() -> TestResult<()> {
    let harness = ConfigIntegrationHarness::new("sce-config-precedence")?;
    let config_path = write_config_file(
        &harness,
        "explicit-config.json",
        r#"{"log_level":"error","timeout_ms":500}"#,
    )?;

    let output = harness
        .base_command(support::sce_binary_path())
        .args([
            OsStr::new("config"),
            OsStr::new("show"),
            OsStr::new("--format"),
            OsStr::new("json"),
            OsStr::new("--config"),
            config_path.as_os_str(),
            OsStr::new("--log-level"),
            OsStr::new("warn"),
            OsStr::new("--timeout-ms"),
            OsStr::new("900"),
        ])
        .env("SCE_LOG_LEVEL", "debug")
        .env("SCE_TIMEOUT_MS", "1200")
        .output()?;

    let result = render_command_result(output);
    assert!(
        result.success(),
        "config show with layered overrides should succeed\nstdout:\n{}\nstderr:\n{}",
        result.stdout,
        result.stderr
    );

    let parsed = parse_json_stdout(&result.stdout)?;
    assert_eq!(parsed["status"], "ok");
    assert_eq!(parsed["result"]["command"], "config_show");
    assert_eq!(
        parsed["result"]["precedence"],
        "flags > env > config file > defaults"
    );
    assert_eq!(parsed["result"]["resolved"]["log_level"]["value"], "warn");
    assert_eq!(parsed["result"]["resolved"]["log_level"]["source"], "flag");
    assert_eq!(parsed["result"]["resolved"]["timeout_ms"]["value"], 900);
    assert_eq!(parsed["result"]["resolved"]["timeout_ms"]["source"], "flag");
    assert_eq!(
        parsed["result"]["resolved"]["log_level"]["config_source"],
        Value::Null
    );
    assert_eq!(
        parsed["result"]["resolved"]["timeout_ms"]["config_source"],
        Value::Null
    );
    assert_eq!(parsed["result"]["config_paths"][0]["source"], "flag");

    Ok(())
}

#[test]
fn config_show_env_overrides_config_when_flags_are_absent() -> TestResult<()> {
    let harness = ConfigIntegrationHarness::new("sce-config-precedence")?;
    let config_path = write_config_file(
        &harness,
        "explicit-config.json",
        r#"{"log_level":"error","timeout_ms":500}"#,
    )?;

    let output = harness
        .base_command(support::sce_binary_path())
        .args([
            OsStr::new("config"),
            OsStr::new("show"),
            OsStr::new("--format"),
            OsStr::new("json"),
            OsStr::new("--config"),
            config_path.as_os_str(),
        ])
        .env("SCE_LOG_LEVEL", "warn")
        .env("SCE_TIMEOUT_MS", "1200")
        .output()?;

    let result = render_command_result(output);
    assert!(
        result.success(),
        "config show with env overrides should succeed\nstdout:\n{}\nstderr:\n{}",
        result.stdout,
        result.stderr
    );

    let parsed = parse_json_stdout(&result.stdout)?;
    assert_eq!(parsed["result"]["resolved"]["log_level"]["value"], "warn");
    assert_eq!(parsed["result"]["resolved"]["log_level"]["source"], "env");
    assert_eq!(parsed["result"]["resolved"]["timeout_ms"]["value"], 1200);
    assert_eq!(parsed["result"]["resolved"]["timeout_ms"]["source"], "env");
    assert_eq!(parsed["result"]["config_paths"][0]["source"], "flag");

    Ok(())
}

#[test]
fn config_show_uses_config_values_when_higher_precedence_inputs_are_absent() -> TestResult<()> {
    let harness = ConfigIntegrationHarness::new("sce-config-precedence")?;
    let config_dir = harness.repo_root().join(".sce");
    fs::create_dir_all(&config_dir)?;
    let config_path = config_dir.join("config.json");
    fs::write(&config_path, r#"{"log_level":"debug","timeout_ms":4567}"#)?;

    let output = harness
        .base_command(support::sce_binary_path())
        .args([
            OsStr::new("config"),
            OsStr::new("show"),
            OsStr::new("--format"),
            OsStr::new("json"),
        ])
        .env_remove("SCE_LOG_LEVEL")
        .env_remove("SCE_TIMEOUT_MS")
        .env_remove("SCE_CONFIG_FILE")
        .output()?;

    let result = render_command_result(output);
    assert!(
        result.success(),
        "config show with discovered config should succeed\nstdout:\n{}\nstderr:\n{}",
        result.stdout,
        result.stderr
    );

    let parsed = parse_json_stdout(&result.stdout)?;
    assert_eq!(parsed["result"]["resolved"]["log_level"]["value"], "debug");
    assert_eq!(
        parsed["result"]["resolved"]["log_level"]["source"],
        "config_file"
    );
    assert_eq!(
        parsed["result"]["resolved"]["log_level"]["config_source"],
        "default_discovered_local"
    );
    assert_eq!(parsed["result"]["resolved"]["timeout_ms"]["value"], 4567);
    assert_eq!(
        parsed["result"]["resolved"]["timeout_ms"]["source"],
        "config_file"
    );
    assert_eq!(
        parsed["result"]["resolved"]["timeout_ms"]["config_source"],
        "default_discovered_local"
    );
    assert_eq!(
        parsed["result"]["config_paths"][0]["path"],
        config_path.display().to_string()
    );
    assert_eq!(
        parsed["result"]["config_paths"][0]["source"],
        "default_discovered_local"
    );

    Ok(())
}

#[test]
fn config_show_uses_defaults_when_no_higher_precedence_inputs_exist() -> TestResult<()> {
    let harness = ConfigIntegrationHarness::new("sce-config-precedence")?;

    let output = harness
        .base_command(support::sce_binary_path())
        .args([
            OsStr::new("config"),
            OsStr::new("show"),
            OsStr::new("--format"),
            OsStr::new("json"),
        ])
        .env_remove("SCE_LOG_LEVEL")
        .env_remove("SCE_TIMEOUT_MS")
        .env_remove("SCE_CONFIG_FILE")
        .output()?;

    let result = render_command_result(output);
    assert!(
        result.success(),
        "config show with no overrides should succeed\nstdout:\n{}\nstderr:\n{}",
        result.stdout,
        result.stderr
    );

    let parsed = parse_json_stdout(&result.stdout)?;
    assert_eq!(parsed["result"]["resolved"]["log_level"]["value"], "info");
    assert_eq!(
        parsed["result"]["resolved"]["log_level"]["source"],
        "default"
    );
    assert_eq!(parsed["result"]["resolved"]["timeout_ms"]["value"], 30000);
    assert_eq!(
        parsed["result"]["resolved"]["timeout_ms"]["source"],
        "default"
    );
    assert_eq!(parsed["result"]["config_paths"], Value::Array(Vec::new()));

    Ok(())
}

fn write_config_file(
    harness: &ConfigIntegrationHarness,
    relative_path: &str,
    contents: &str,
) -> TestResult<std::path::PathBuf> {
    let config_path = harness.temp_path().join(relative_path);
    fs::write(&config_path, contents)?;
    Ok(config_path)
}

fn parse_json_stdout(stdout: &str) -> TestResult<Value> {
    Ok(serde_json::from_str(stdout)?)
}
