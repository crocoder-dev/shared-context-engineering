use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context};
use serde_json::Value;

use crate::services::agent_trace::sce_config_schema_url;
use crate::services::config::schema::{parse_file_config, CONFIG_SCHEMA_DECLARATION_KEY};
use crate::services::config::schema_declaration::{
    classify_schema_declaration, current_schema_series, SchemaDeclaration, SchemaVersion,
};

use super::config_file_write::{read_optional, write_guarded, ConfigLayer, GuardedWriteError};

#[derive(Debug)]
pub(crate) enum SchemaUpgradePlan {
    NotPresent,
    NoDeclaration,
    Current,
    NewerSeries(SchemaVersion),
    Unrecognized,
    Unreadable(anyhow::Error),
    Ready {
        original: Vec<u8>,
        candidate: Vec<u8>,
        from: SchemaVersion,
    },
    CandidateInvalid {
        from: SchemaVersion,
        reason: String,
    },
}

#[derive(Debug)]
pub(crate) enum SchemaUpgradeOutcome {
    Upgraded { from: SchemaVersion },
    Unchanged,
    Skipped(SchemaUpgradeSkip),
}

#[derive(Debug)]
pub(crate) enum SchemaUpgradeSkip {
    NewerSeries(SchemaVersion),
    Unrecognized,
    CandidateInvalid { from: SchemaVersion, reason: String },
    SymlinkManaged,
    ConcurrentModification,
    Io(anyhow::Error),
}

impl From<GuardedWriteError> for SchemaUpgradeSkip {
    fn from(error: GuardedWriteError) -> Self {
        match error {
            GuardedWriteError::ConcurrentModification => Self::ConcurrentModification,
            GuardedWriteError::SymlinkManaged => Self::SymlinkManaged,
            GuardedWriteError::Io(error) => Self::Io(error),
        }
    }
}

/// Reads and classifies one canonical config file and, for an older-series
/// declaration, builds and validates the `$schema`-only candidate. Never writes.
pub(crate) fn plan_schema_upgrade(path: &Path, layer: ConfigLayer) -> SchemaUpgradePlan {
    let original = match read_optional(path, layer) {
        Ok(Some(bytes)) => bytes,
        Ok(None) => return SchemaUpgradePlan::NotPresent,
        Err(error) => return SchemaUpgradePlan::Unreadable(error),
    };
    let document = match parse_document(path, layer, &original) {
        Ok(document) => document,
        Err(error) => return SchemaUpgradePlan::Unreadable(error),
    };

    let declaration = document
        .value
        .as_object()
        .and_then(|object| object.get(CONFIG_SCHEMA_DECLARATION_KEY));
    let from = match classify_schema_declaration(declaration, current_schema_series()) {
        SchemaDeclaration::Absent => return SchemaUpgradePlan::NoDeclaration,
        SchemaDeclaration::CurrentSeries => return SchemaUpgradePlan::Current,
        SchemaDeclaration::NewerSeries(version) => return SchemaUpgradePlan::NewerSeries(version),
        SchemaDeclaration::Unrecognized => return SchemaUpgradePlan::Unrecognized,
        SchemaDeclaration::OlderSeries(version) => version,
    };

    let candidate = match build_candidate(&document, &sce_config_schema_url()) {
        Ok(candidate) => candidate,
        Err(error) => return SchemaUpgradePlan::Unreadable(error),
    };
    if let Err(error) = parse_file_config(&candidate, path, layer.path_source()) {
        return SchemaUpgradePlan::CandidateInvalid {
            from,
            reason: format!("{error:#}"),
        };
    }

    SchemaUpgradePlan::Ready {
        original,
        candidate: candidate.into_bytes(),
        from,
    }
}

pub(crate) fn commit_schema_upgrade(
    path: &Path,
    layer: ConfigLayer,
    plan: SchemaUpgradePlan,
) -> SchemaUpgradeOutcome {
    commit_schema_upgrade_with(path, layer, plan, |path| read_optional(path, layer))
}

fn commit_schema_upgrade_with<R>(
    path: &Path,
    layer: ConfigLayer,
    plan: SchemaUpgradePlan,
    reread: R,
) -> SchemaUpgradeOutcome
where
    R: FnOnce(&Path) -> anyhow::Result<Option<Vec<u8>>>,
{
    let skip = match plan {
        SchemaUpgradePlan::NotPresent
        | SchemaUpgradePlan::NoDeclaration
        | SchemaUpgradePlan::Current => return SchemaUpgradeOutcome::Unchanged,
        SchemaUpgradePlan::NewerSeries(version) => SchemaUpgradeSkip::NewerSeries(version),
        SchemaUpgradePlan::Unrecognized => SchemaUpgradeSkip::Unrecognized,
        SchemaUpgradePlan::Unreadable(error) => SchemaUpgradeSkip::Io(error),
        SchemaUpgradePlan::CandidateInvalid { from, reason } => {
            SchemaUpgradeSkip::CandidateInvalid { from, reason }
        }
        SchemaUpgradePlan::Ready {
            original,
            candidate,
            from,
        } => match write_guarded(path, layer, Some(&original), &candidate, reread) {
            Ok(()) => return SchemaUpgradeOutcome::Upgraded { from },
            Err(error) => error.into(),
        },
    };
    SchemaUpgradeOutcome::Skipped(skip)
}

#[derive(Debug)]
pub(crate) struct SchemaUpgradeReport {
    pub(crate) path: PathBuf,
    pub(crate) outcome: SchemaUpgradeOutcome,
}

#[derive(Debug)]
struct PlannedSchemaUpgrade {
    path: PathBuf,
    layer: ConfigLayer,
    plan: SchemaUpgradePlan,
}

#[derive(Debug, Default)]
pub(crate) struct CanonicalSchemaUpgrades {
    files: Vec<PlannedSchemaUpgrade>,
}

impl CanonicalSchemaUpgrades {
    pub(crate) fn plan(repository_config_path: &Path, global_config_path: Option<&Path>) -> Self {
        let files = [
            Some((repository_config_path, ConfigLayer::Repository)),
            global_config_path.map(|path| (path, ConfigLayer::Global)),
        ]
        .into_iter()
        .flatten()
        .map(|(path, layer)| PlannedSchemaUpgrade {
            path: path.to_path_buf(),
            layer,
            plan: plan_schema_upgrade(path, layer),
        })
        .collect();
        Self { files }
    }

    pub(crate) fn candidate(&self, path: &Path) -> Option<&[u8]> {
        self.files.iter().find_map(|file| match &file.plan {
            SchemaUpgradePlan::Ready { candidate, .. } if file.path == path => {
                Some(candidate.as_slice())
            }
            _ => None,
        })
    }

    pub(crate) fn commit(self) -> Vec<SchemaUpgradeReport> {
        self.files
            .into_iter()
            .map(|file| SchemaUpgradeReport {
                outcome: commit_schema_upgrade(&file.path, file.layer, file.plan),
                path: file.path,
            })
            .collect()
    }
}

struct ConfigDocument<'a> {
    raw: &'a str,
    value: Value,
}

fn parse_document<'a>(
    path: &Path,
    layer: ConfigLayer,
    bytes: &'a [u8],
) -> anyhow::Result<ConfigDocument<'a>> {
    let raw = std::str::from_utf8(bytes)
        .with_context(|| format!("{} '{}' is not valid UTF-8", layer.label(), path.display()))?;
    let value: Value = serde_json::from_str(raw)
        .with_context(|| format!("{} '{}' is not valid JSON", layer.label(), path.display()))?;
    if !value.is_object() {
        return Err(anyhow!(
            "{} '{}' must contain a top-level JSON object",
            layer.label(),
            path.display()
        ));
    }
    Ok(ConfigDocument { raw, value })
}

/// Prefers swapping the single encoded `$schema` literal so every other byte is
/// preserved; the swap is kept only when the result is structurally identical
/// to the original apart from `$schema`.
fn build_candidate(document: &ConfigDocument<'_>, target_url: &str) -> anyhow::Result<String> {
    let mut expected = document.value.clone();
    let object = expected
        .as_object_mut()
        .context("config document must be a JSON object")?;
    let previous = object.insert(
        CONFIG_SCHEMA_DECLARATION_KEY.to_string(),
        Value::String(target_url.to_string()),
    );

    if let Some(Value::String(previous_url)) = previous {
        let previous_literal = serde_json::to_string(&previous_url)?;
        if document.raw.matches(&previous_literal).count() == 1 {
            let target_literal = serde_json::to_string(target_url)?;
            let swapped = document.raw.replacen(&previous_literal, &target_literal, 1);
            if serde_json::from_str::<Value>(&swapped).ok().as_ref() == Some(&expected) {
                return Ok(swapped);
            }
        }
    }

    let mut serialized = serde_json::to_string_pretty(&expected)?;
    serialized.push('\n');
    Ok(serialized)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::json;

    use super::*;
    use crate::services::config::schema_declaration::SchemaSeries;
    use crate::services::config::ConfigPathSource;

    fn schema_url(version: &str) -> String {
        format!("https://sce.crocoder.dev/v{version}/config.json")
    }

    fn older_series_version() -> String {
        let SchemaSeries { major, minor } = current_schema_series();
        if minor > 0 {
            format!("{major}.{}.3", minor - 1)
        } else {
            format!("{}.9.3", major - 1)
        }
    }

    fn newer_series_version() -> String {
        let SchemaSeries { major, minor } = current_schema_series();
        format!("{major}.{}.0", minor + 1)
    }

    fn config_path(dir: &Path) -> PathBuf {
        dir.join(".sce").join("config.json")
    }

    fn write_config(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    fn stale_config(declaration: &str) -> String {
        format!(
            r#"{{
    "$schema":   "{declaration}",
  "auth": {{ "mode": "disabled" }},
  "telemetry": {{"enabled": false}},
  "integrations": {{
    "target": ["claude",   "opencode"]
  }},
  "policies": {{
    "bash": {{
      "custom": [
        {{ "id": "no-curl", "match": {{ "argv_prefix": ["curl"] }}, "message": "Use the fetch helper." }}
      ]
    }}
  }}
}}
"#
        )
    }

    fn plan_and_commit(path: &Path, layer: ConfigLayer) -> SchemaUpgradeOutcome {
        commit_schema_upgrade(path, layer, plan_schema_upgrade(path, layer))
    }

    #[test]
    fn schema_upgrade_ignores_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_path(dir.path());

        assert!(matches!(
            plan_schema_upgrade(&path, ConfigLayer::Global),
            SchemaUpgradePlan::NotPresent
        ));
        assert!(matches!(
            plan_and_commit(&path, ConfigLayer::Global),
            SchemaUpgradeOutcome::Unchanged
        ));
        assert!(!path.exists());
    }

    #[test]
    fn schema_upgrade_leaves_file_without_declaration_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_path(dir.path());
        let contents = "{ \"auth\": { \"mode\": \"optional\" } }\n";
        write_config(&path, contents);

        assert!(matches!(
            plan_schema_upgrade(&path, ConfigLayer::Repository),
            SchemaUpgradePlan::NoDeclaration
        ));
        assert!(matches!(
            plan_and_commit(&path, ConfigLayer::Repository),
            SchemaUpgradeOutcome::Unchanged
        ));
        assert_eq!(fs::read_to_string(&path).unwrap(), contents);
    }

    #[test]
    fn schema_upgrade_preserves_same_series_declarations() {
        let SchemaSeries { major, minor } = current_schema_series();
        let dir = tempfile::tempdir().unwrap();
        let path = config_path(dir.path());
        for version in [
            format!("{major}.{minor}.0-pre-alpha-v1"),
            format!("{major}.{minor}.7"),
            format!("{major}.{minor}.7+build.5"),
        ] {
            let contents = stale_config(&schema_url(&version));
            write_config(&path, &contents);

            assert!(matches!(
                plan_schema_upgrade(&path, ConfigLayer::Repository),
                SchemaUpgradePlan::Current
            ));
            assert!(matches!(
                plan_and_commit(&path, ConfigLayer::Repository),
                SchemaUpgradeOutcome::Unchanged
            ));
            assert_eq!(fs::read_to_string(&path).unwrap(), contents);
        }
    }

    #[test]
    fn schema_upgrade_replaces_only_older_series_declaration() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_path(dir.path());
        for older in [older_series_version(), "0.1.0".to_string()] {
            let contents = stale_config(&schema_url(&older));
            write_config(&path, &contents);

            let outcome = plan_and_commit(&path, ConfigLayer::Repository);

            let SchemaUpgradeOutcome::Upgraded { from } = outcome else {
                panic!("expected an upgrade, got {outcome:?}");
            };
            assert_eq!(from.to_string(), older);
            assert_eq!(
                fs::read_to_string(&path).unwrap(),
                stale_config(&sce_config_schema_url())
            );
            assert!(matches!(
                plan_schema_upgrade(&path, ConfigLayer::Repository),
                SchemaUpgradePlan::Current
            ));
            let upgraded = fs::read(&path).unwrap();
            assert!(matches!(
                plan_and_commit(&path, ConfigLayer::Repository),
                SchemaUpgradeOutcome::Unchanged
            ));
            assert_eq!(fs::read(&path).unwrap(), upgraded);
        }
    }

    #[test]
    fn schema_upgrade_falls_back_to_pretty_serialization_when_literal_is_ambiguous() {
        let older_url = schema_url(&older_series_version());
        let raw = format!(
            "{{\"$schema\":\"{older_url}\",\"policies\":{{\"bash\":{{\"custom\":[{{\"id\":\"x\",\"match\":{{\"argv_prefix\":[\"{older_url}\"]}},\"message\":\"m\"}}]}}}}}}"
        );
        let document = ConfigDocument {
            raw: &raw,
            value: serde_json::from_str(&raw).unwrap(),
        };

        let candidate = build_candidate(&document, &sce_config_schema_url()).unwrap();

        let candidate_value: Value = serde_json::from_str(&candidate).unwrap();
        assert_eq!(candidate_value["$schema"], json!(sce_config_schema_url()));
        assert_eq!(
            candidate_value["policies"]["bash"]["custom"][0]["match"]["argv_prefix"],
            json!([older_url])
        );
        assert!(candidate.ends_with("}\n"));
    }

    #[test]
    fn schema_upgrade_never_rewrites_newer_or_unrecognized_declarations() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_path(dir.path());

        let newer = stale_config(&schema_url(&newer_series_version()));
        write_config(&path, &newer);
        assert!(matches!(
            plan_and_commit(&path, ConfigLayer::Repository),
            SchemaUpgradeOutcome::Skipped(SchemaUpgradeSkip::NewerSeries(version))
                if version.to_string() == newer_series_version()
        ));
        assert_eq!(fs::read_to_string(&path).unwrap(), newer);

        for foreign in [
            "https://example.com/s.json".to_string(),
            "https://sce.crocoder.dev/config.json".to_string(),
            schema_url("0.4"),
            format!(
                "http://sce.crocoder.dev/v{}/config.json",
                older_series_version()
            ),
        ] {
            let contents = stale_config(&foreign);
            write_config(&path, &contents);
            assert!(matches!(
                plan_and_commit(&path, ConfigLayer::Repository),
                SchemaUpgradeOutcome::Skipped(SchemaUpgradeSkip::Unrecognized)
            ));
            assert_eq!(fs::read_to_string(&path).unwrap(), contents);
        }

        let non_string = "{ \"$schema\": 4 }\n";
        write_config(&path, non_string);
        assert!(matches!(
            plan_and_commit(&path, ConfigLayer::Repository),
            SchemaUpgradeOutcome::Skipped(SchemaUpgradeSkip::Unrecognized)
        ));
        assert_eq!(fs::read_to_string(&path).unwrap(), non_string);
    }

    #[test]
    fn schema_upgrade_reports_unreadable_files_without_writing() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_path(dir.path());
        for invalid in [b"{ not json".to_vec(), b"[]\n".to_vec(), vec![0xff, 0xfe]] {
            write_config(&path, "");
            fs::write(&path, &invalid).unwrap();

            assert!(matches!(
                plan_schema_upgrade(&path, ConfigLayer::Repository),
                SchemaUpgradePlan::Unreadable(_)
            ));
            assert!(matches!(
                plan_and_commit(&path, ConfigLayer::Repository),
                SchemaUpgradeOutcome::Skipped(SchemaUpgradeSkip::Io(_))
            ));
            assert_eq!(fs::read(&path).unwrap(), invalid);
        }
    }

    #[test]
    fn schema_upgrade_keeps_file_when_candidate_fails_validation() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_path(dir.path());
        let older = older_series_version();
        let contents = format!(
            "{{\n  \"$schema\": \"{}\",\n  \"unknown_key\": true\n}}\n",
            schema_url(&older)
        );
        write_config(&path, &contents);

        let plan = plan_schema_upgrade(&path, ConfigLayer::Repository);
        let SchemaUpgradePlan::CandidateInvalid { from, reason } = &plan else {
            panic!("expected an invalid candidate, got {plan:?}");
        };
        assert_eq!(from.to_string(), older);
        assert!(reason.contains("unknown_key"), "{reason}");
        assert!(!reason.contains("/$schema"), "{reason}");

        assert!(matches!(
            commit_schema_upgrade(&path, ConfigLayer::Repository, plan),
            SchemaUpgradeOutcome::Skipped(SchemaUpgradeSkip::CandidateInvalid { .. })
        ));
        assert_eq!(fs::read_to_string(&path).unwrap(), contents);
    }

    #[test]
    fn schema_upgrade_validates_global_candidate_as_discovered_global() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sce").join("config.json");
        let contents = stale_config(&schema_url(&older_series_version()));
        write_config(&path, &contents);

        let SchemaUpgradePlan::Ready { candidate, .. } =
            plan_schema_upgrade(&path, ConfigLayer::Global)
        else {
            panic!("expected a ready global plan");
        };
        let parsed = parse_file_config(
            std::str::from_utf8(&candidate).unwrap(),
            &path,
            ConfigLayer::Global.path_source(),
        )
        .unwrap();
        assert_eq!(
            parsed.auth_mode.unwrap().source,
            ConfigPathSource::DefaultDiscoveredGlobal
        );

        assert!(matches!(
            plan_and_commit(&path, ConfigLayer::Global),
            SchemaUpgradeOutcome::Upgraded { .. }
        ));
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            stale_config(&sce_config_schema_url())
        );
    }

    #[test]
    fn schema_upgrade_refuses_concurrent_modification_between_plan_and_commit() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_path(dir.path());
        write_config(&path, &stale_config(&schema_url(&older_series_version())));
        let plan = plan_schema_upgrade(&path, ConfigLayer::Repository);
        assert!(matches!(plan, SchemaUpgradePlan::Ready { .. }));
        let concurrent = stale_config(&schema_url("0.1.0"));

        let outcome = commit_schema_upgrade_with(&path, ConfigLayer::Repository, plan, |path| {
            fs::write(path, &concurrent).unwrap();
            read_optional(path, ConfigLayer::Repository)
        });

        assert!(matches!(
            outcome,
            SchemaUpgradeOutcome::Skipped(SchemaUpgradeSkip::ConcurrentModification)
        ));
        assert_eq!(fs::read_to_string(&path).unwrap(), concurrent);
    }

    #[cfg(unix)]
    #[test]
    fn schema_upgrade_refuses_symlinked_file_and_leaves_it_unchanged() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let path = config_path(dir.path());
        let target = dir.path().join("dotfiles-config.json");
        let contents = stale_config(&schema_url(&older_series_version()));
        fs::write(&target, &contents).unwrap();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        symlink(&target, &path).unwrap();

        assert!(matches!(
            plan_and_commit(&path, ConfigLayer::Global),
            SchemaUpgradeOutcome::Skipped(SchemaUpgradeSkip::SymlinkManaged)
        ));
        assert_eq!(fs::read_link(&path).unwrap(), target);
        assert_eq!(fs::read_to_string(&target).unwrap(), contents);
    }

    #[cfg(unix)]
    #[test]
    fn schema_upgrade_leaves_dangling_symlink_untouched() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let path = config_path(dir.path());
        let target = dir.path().join("missing-target.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        symlink(&target, &path).unwrap();

        assert!(matches!(
            plan_and_commit(&path, ConfigLayer::Global),
            SchemaUpgradeOutcome::Unchanged
        ));
        assert_eq!(fs::read_link(&path).unwrap(), target);
        assert!(!target.exists());
    }

    #[cfg(unix)]
    #[test]
    fn schema_upgrade_replaces_file_atomically_and_keeps_its_mode() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = config_path(dir.path());
        write_config(&path, &stale_config(&schema_url(&older_series_version())));
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();

        assert!(matches!(
            plan_and_commit(&path, ConfigLayer::Repository),
            SchemaUpgradeOutcome::Upgraded { .. }
        ));

        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o7777,
            0o640
        );
        let entries: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(entries, vec![std::ffi::OsString::from("config.json")]);
    }
}
