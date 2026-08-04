//! `FilesystemIntegrationConfigRepository`: the `IntegrationConfigRepository`
//! outbound adapter that owns repo-local `.sce/config.json` lifecycle, JSON
//! parsing/merge, and serialization, porting persistence previously inline
//! in `services::setup`.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde_json::json;

use crate::application::ports::integration_config_repository::IntegrationConfigRepository;
use crate::domain::integration::IntegrationTarget;
use crate::services::agent_trace::SCE_WEB_BASE_URL;
use crate::services::default_paths::RepoPaths;

/// Canonical JSON payload for a newly bootstrapped repo-local
/// `.sce/config.json`. Contains only the `$schema` declaration pointing to
/// the SCE config JSON Schema.
fn repo_local_config_bootstrap_payload() -> String {
    format!("{{\n  \"$schema\": \"{SCE_WEB_BASE_URL}/config.json\"\n}}\n")
}

fn read_config_document(config_file: &Path) -> Result<serde_json::Value> {
    let raw = fs::read_to_string(config_file)
        .with_context(|| format!("Failed to read config file '{}'", config_file.display()))?;

    let document: serde_json::Value = serde_json::from_str(&raw).with_context(|| {
        format!(
            "Config file '{}' must contain valid JSON.",
            config_file.display()
        )
    })?;

    Ok(document)
}

fn recorded_string_list(document: &serde_json::Value, field: &str) -> Vec<String> {
    document
        .get("integrations")
        .and_then(|integrations| integrations.get(field))
        .and_then(|value| value.as_array())
        .map(|values| {
            values
                .iter()
                .filter_map(|value| value.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

/// Owns repo-local integration configuration persistence directly on disk:
/// bootstrap, optional-workflow reads, and recording concrete installed
/// targets.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct FilesystemIntegrationConfigRepository;

impl IntegrationConfigRepository for FilesystemIntegrationConfigRepository {
    type Error = anyhow::Error;

    fn ensure_exists(&self, repository_root: &Path) -> Result<()> {
        let repo_paths = RepoPaths::new(repository_root);
        let config_file = repo_paths.sce_config_file();

        if config_file.exists() {
            return Ok(());
        }

        let sce_dir = repo_paths.sce_dir();
        fs::create_dir_all(&sce_dir).with_context(|| {
            format!(
                "Failed to create repo-local config directory '{}'",
                sce_dir.display()
            )
        })?;

        fs::write(&config_file, repo_local_config_bootstrap_payload()).with_context(|| {
            format!(
                "Failed to write repo-local config file '{}'",
                config_file.display()
            )
        })?;

        Ok(())
    }

    fn load_optional_workflows(&self, repository_root: &Path) -> Result<Vec<String>> {
        let config_file = RepoPaths::new(repository_root).sce_config_file();
        let document = read_config_document(&config_file)?;

        if !document.is_object() {
            anyhow::bail!(
                "Config file '{}' must contain a top-level JSON object.",
                config_file.display()
            );
        }

        Ok(recorded_string_list(&document, "optional_workflows"))
    }

    fn record_installation(
        &self,
        repository_root: &Path,
        targets: &[IntegrationTarget],
        optional_workflows: &[String],
    ) -> Result<()> {
        let repo_paths = RepoPaths::new(repository_root);
        let config_file = repo_paths.sce_config_file();

        if !config_file.exists() {
            self.ensure_exists(repository_root)?;
        }

        let mut document = read_config_document(&config_file)?;
        let mut existing_targets = recorded_string_list(&document, "target");

        let config_obj = document.as_object_mut().with_context(|| {
            format!(
                "Config file '{}' must contain a top-level JSON object.",
                config_file.display()
            )
        })?;

        for target in targets {
            let id = target.config_id().to_string();
            if !existing_targets.contains(&id) {
                existing_targets.push(id);
            }
        }

        config_obj.insert(
            "integrations".to_string(),
            json!({
                "target": existing_targets,
                "optional_workflows": optional_workflows,
            }),
        );

        let updated = serde_json::to_string_pretty(&document).with_context(|| {
            format!(
                "Failed to serialize updated config for '{}'",
                config_file.display()
            )
        })? + "\n";

        fs::write(&config_file, updated)
            .with_context(|| format!("Failed to write config file '{}'", config_file.display()))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(label: &str) -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after Unix epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "sce-filesystem-integration-config-repository-{label}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn ensure_exists_creates_a_missing_config_with_the_canonical_payload() {
        let repo = unique_temp_dir("ensure-missing");
        let repository = FilesystemIntegrationConfigRepository;

        repository.ensure_exists(&repo).expect("ensure exists");

        let config_file = RepoPaths::new(&repo).sce_config_file();
        let content = fs::read_to_string(&config_file).expect("read config");
        assert_eq!(content, repo_local_config_bootstrap_payload());
        assert!(content.ends_with('\n') && !content.ends_with("\n\n"));
    }

    #[test]
    fn ensure_exists_never_overwrites_an_existing_config() {
        let repo = unique_temp_dir("ensure-existing");
        let repository = FilesystemIntegrationConfigRepository;
        let config_file = RepoPaths::new(&repo).sce_config_file();

        fs::create_dir_all(config_file.parent().unwrap()).expect("seed sce dir");
        fs::write(&config_file, "{\"sentinel\": true}\n").expect("seed config");

        repository.ensure_exists(&repo).expect("ensure exists");

        let content = fs::read_to_string(&config_file).expect("read config");
        assert_eq!(content, "{\"sentinel\": true}\n");
    }

    #[test]
    fn record_installation_bootstraps_a_missing_config_before_recording() {
        let repo = unique_temp_dir("record-missing");
        let repository = FilesystemIntegrationConfigRepository;

        repository
            .record_installation(&repo, &[IntegrationTarget::Claude], &[])
            .expect("record installation");

        let config_file = RepoPaths::new(&repo).sce_config_file();
        let document: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&config_file).expect("read config"))
                .expect("valid json");

        assert_eq!(
            document["$schema"],
            json!(format!("{SCE_WEB_BASE_URL}/config.json"))
        );
        assert_eq!(document["integrations"]["target"], json!(["claude"]));
        assert_eq!(document["integrations"]["optional_workflows"], json!([]));
    }

    #[test]
    fn record_installation_preserves_unrelated_top_level_fields() {
        let repo = unique_temp_dir("record-unrelated");
        let repository = FilesystemIntegrationConfigRepository;
        let config_file = RepoPaths::new(&repo).sce_config_file();
        fs::create_dir_all(config_file.parent().unwrap()).expect("seed sce dir");
        fs::write(&config_file, "{\"custom\": \"value\"}\n").expect("seed config");

        repository
            .record_installation(&repo, &[IntegrationTarget::OpenCode], &[])
            .expect("record installation");

        let document: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&config_file).expect("read config"))
                .expect("valid json");
        assert_eq!(document["custom"], json!("value"));
    }

    #[test]
    fn record_installation_preserves_existing_target_order_and_dedups_new_targets() {
        let repo = unique_temp_dir("record-order");
        let repository = FilesystemIntegrationConfigRepository;
        let config_file = RepoPaths::new(&repo).sce_config_file();
        fs::create_dir_all(config_file.parent().unwrap()).expect("seed sce dir");
        fs::write(
            &config_file,
            "{\"integrations\": {\"target\": [\"claude\", \"pi\"]}}\n",
        )
        .expect("seed config");

        repository
            .record_installation(
                &repo,
                &[IntegrationTarget::Pi, IntegrationTarget::OpenCode],
                &[],
            )
            .expect("record installation");

        let document: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&config_file).expect("read config"))
                .expect("valid json");
        assert_eq!(
            document["integrations"]["target"],
            json!(["claude", "pi", "opencode"])
        );
    }

    #[test]
    fn record_installation_preserves_unknown_existing_target_strings() {
        let repo = unique_temp_dir("record-unknown");
        let repository = FilesystemIntegrationConfigRepository;
        let config_file = RepoPaths::new(&repo).sce_config_file();
        fs::create_dir_all(config_file.parent().unwrap()).expect("seed sce dir");
        fs::write(
            &config_file,
            "{\"integrations\": {\"target\": [\"future-target\"]}}\n",
        )
        .expect("seed config");

        repository
            .record_installation(&repo, &[IntegrationTarget::Claude], &[])
            .expect("record installation");

        let document: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&config_file).expect("read config"))
                .expect("valid json");
        assert_eq!(
            document["integrations"]["target"],
            json!(["future-target", "claude"])
        );
    }

    #[test]
    fn record_installation_replaces_optional_workflows_with_the_current_selection() {
        let repo = unique_temp_dir("record-workflows");
        let repository = FilesystemIntegrationConfigRepository;
        let config_file = RepoPaths::new(&repo).sce_config_file();
        fs::create_dir_all(config_file.parent().unwrap()).expect("seed sce dir");
        fs::write(
            &config_file,
            "{\"integrations\": {\"optional_workflows\": [\"stale\"]}}\n",
        )
        .expect("seed config");

        repository
            .record_installation(
                &repo,
                &[IntegrationTarget::Claude],
                &["research".to_string()],
            )
            .expect("record installation");

        let document: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&config_file).expect("read config"))
                .expect("valid json");
        assert_eq!(
            document["integrations"]["optional_workflows"],
            json!(["research"])
        );
    }

    #[test]
    fn record_installation_returns_an_error_for_invalid_json() {
        let repo = unique_temp_dir("record-invalid-json");
        let repository = FilesystemIntegrationConfigRepository;
        let config_file = RepoPaths::new(&repo).sce_config_file();
        fs::create_dir_all(config_file.parent().unwrap()).expect("seed sce dir");
        fs::write(&config_file, "not json").expect("seed config");

        let error = repository
            .record_installation(&repo, &[IntegrationTarget::Claude], &[])
            .unwrap_err();

        assert!(error.to_string().contains("must contain valid JSON"));
    }

    #[test]
    fn record_installation_returns_an_error_for_a_non_object_top_level_value() {
        let repo = unique_temp_dir("record-non-object");
        let repository = FilesystemIntegrationConfigRepository;
        let config_file = RepoPaths::new(&repo).sce_config_file();
        fs::create_dir_all(config_file.parent().unwrap()).expect("seed sce dir");
        fs::write(&config_file, "[]\n").expect("seed config");

        let error = repository
            .record_installation(&repo, &[IntegrationTarget::Claude], &[])
            .unwrap_err();

        assert!(error
            .to_string()
            .contains("must contain a top-level JSON object"));
    }

    #[test]
    fn record_installation_writes_pretty_json_with_exactly_one_final_newline() {
        let repo = unique_temp_dir("record-newline");
        let repository = FilesystemIntegrationConfigRepository;

        repository
            .record_installation(&repo, &[IntegrationTarget::Claude], &[])
            .expect("record installation");

        let config_file = RepoPaths::new(&repo).sce_config_file();
        let content = fs::read_to_string(&config_file).expect("read config");

        assert!(content.ends_with('\n') && !content.ends_with("\n\n"));
        assert!(content.contains("\n  "), "expected pretty-printed JSON");
    }

    #[test]
    fn load_optional_workflows_returns_the_recorded_workflows() {
        let repo = unique_temp_dir("load-recorded");
        let repository = FilesystemIntegrationConfigRepository;
        let config_file = RepoPaths::new(&repo).sce_config_file();
        fs::create_dir_all(config_file.parent().unwrap()).expect("seed sce dir");
        fs::write(
            &config_file,
            "{\"integrations\": {\"optional_workflows\": [\"research\", \"docs\"]}}\n",
        )
        .expect("seed config");

        let workflows = repository
            .load_optional_workflows(&repo)
            .expect("load optional workflows");

        assert_eq!(workflows, vec!["research".to_string(), "docs".to_string()]);
    }

    #[test]
    fn load_optional_workflows_returns_empty_when_none_are_recorded() {
        let repo = unique_temp_dir("load-empty");
        let repository = FilesystemIntegrationConfigRepository;
        let config_file = RepoPaths::new(&repo).sce_config_file();
        fs::create_dir_all(config_file.parent().unwrap()).expect("seed sce dir");
        fs::write(&config_file, "{}\n").expect("seed config");

        let workflows = repository
            .load_optional_workflows(&repo)
            .expect("load optional workflows");

        assert!(workflows.is_empty());
    }

    #[test]
    fn load_optional_workflows_returns_an_error_when_the_config_is_missing() {
        let repo = unique_temp_dir("load-missing");
        let repository = FilesystemIntegrationConfigRepository;

        let error = repository.load_optional_workflows(&repo).unwrap_err();

        assert!(error.to_string().contains("Failed to read config file"));
    }

    #[test]
    fn load_optional_workflows_returns_an_error_for_invalid_json() {
        let repo = unique_temp_dir("load-invalid-json");
        let repository = FilesystemIntegrationConfigRepository;
        let config_file = RepoPaths::new(&repo).sce_config_file();
        fs::create_dir_all(config_file.parent().unwrap()).expect("seed sce dir");
        fs::write(&config_file, "not json").expect("seed config");

        let error = repository.load_optional_workflows(&repo).unwrap_err();

        assert!(error.to_string().contains("must contain valid JSON"));
    }
}
