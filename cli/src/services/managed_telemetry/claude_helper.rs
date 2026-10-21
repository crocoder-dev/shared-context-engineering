use std::path::Path;

use super::{
    MANAGED_OTLP_ENDPOINT, MANAGED_OTLP_LOGS_ENDPOINT, MANAGED_OTLP_METRICS_ENDPOINT,
    MANAGED_OTLP_TRACES_ENDPOINT,
};
use crate::services::config;
use crate::services::default_paths::enclosing_repository_root;
use crate::services::repository_identity::resolve::resolve_repository_identity;

const DESTINATION_ENVS: [(&str, &str); 5] = [
    ("OTEL_EXPORTER_OTLP_ENDPOINT", MANAGED_OTLP_ENDPOINT),
    (
        "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
        MANAGED_OTLP_TRACES_ENDPOINT,
    ),
    (
        "OTEL_EXPORTER_OTLP_METRICS_ENDPOINT",
        MANAGED_OTLP_METRICS_ENDPOINT,
    ),
    (
        "OTEL_EXPORTER_OTLP_LOGS_ENDPOINT",
        MANAGED_OTLP_LOGS_ENDPOINT,
    ),
    ("BETA_TRACING_ENDPOINT", MANAGED_OTLP_ENDPOINT),
];

fn normalized(value: &str) -> &str {
    value.trim().trim_end_matches('/')
}

pub fn visible_destination_conflict<F>(lookup: F) -> bool
where
    F: Fn(&str) -> Option<String>,
{
    DESTINATION_ENVS
        .iter()
        .any(|(key, expected)| lookup(key).is_some_and(|value| normalized(&value) != *expected))
}

pub fn repository_id_for(working_dir: &Path) -> Option<String> {
    let root = enclosing_repository_root(working_dir)?;
    let storage_config = config::resolve_agent_trace_storage_runtime_config(root).ok()?;
    resolve_repository_identity(
        root,
        storage_config.repository_id.as_deref(),
        &storage_config.repository_remote,
    )
    .ok()
    .map(|resolved| resolved.identity.repository_id)
}

#[cfg(test)]
pub(crate) mod tests {
    use std::collections::HashMap;
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;

    use super::*;

    fn lookup_from(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map = pairs
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect::<HashMap<_, _>>();
        move |key| map.get(key).cloned()
    }

    pub(crate) fn git_repository_with_remote(parent: &Path, name: &str, remote: &str) -> PathBuf {
        let root = parent.join(name);
        fs::create_dir_all(root.join("src").join("module")).unwrap();
        for args in [
            vec!["init", "--quiet"],
            vec!["remote", "add", "origin", remote],
        ] {
            let status = Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(args)
                .status()
                .unwrap();
            assert!(status.success());
        }
        root
    }

    #[test]
    fn helper_without_any_visible_otel_variables_is_not_a_conflict() {
        assert!(!visible_destination_conflict(lookup_from(&[])));
        assert!(!visible_destination_conflict(lookup_from(&[
            (
                "OTEL_EXPORTER_OTLP_ENDPOINT",
                "https://otel.crocoderlab.dev/"
            ),
            (
                "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
                MANAGED_OTLP_TRACES_ENDPOINT
            ),
            ("BETA_TRACING_ENDPOINT", MANAGED_OTLP_ENDPOINT),
        ])));
    }

    #[test]
    fn helper_refuses_visible_conflicting_destinations() {
        for (key, value) in [
            ("OTEL_EXPORTER_OTLP_ENDPOINT", "http://localhost:4318"),
            (
                "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
                "https://other.example/v1/traces",
            ),
            (
                "OTEL_EXPORTER_OTLP_METRICS_ENDPOINT",
                MANAGED_OTLP_LOGS_ENDPOINT,
            ),
            ("OTEL_EXPORTER_OTLP_LOGS_ENDPOINT", ""),
            ("BETA_TRACING_ENDPOINT", "https://collector.example"),
        ] {
            assert!(
                visible_destination_conflict(lookup_from(&[(key, value)])),
                "{key}={value}"
            );
        }
    }

    #[test]
    fn helper_resolves_repository_identity_from_nested_launch_directory() {
        let dir = tempfile::tempdir().unwrap();
        let first =
            git_repository_with_remote(dir.path(), "first", "git@github.com:acme/first.git");
        let second =
            git_repository_with_remote(dir.path(), "second", "https://github.com/acme/second");

        let root_id = repository_id_for(&first).expect("repository identity");
        assert_eq!(
            repository_id_for(&first.join("src").join("module")),
            Some(root_id.clone())
        );
        assert_ne!(repository_id_for(&second), Some(root_id));
        assert_eq!(repository_id_for(&dir.path().join("not-a-repo")), None);

        let without_remote = dir.path().join("no-remote");
        fs::create_dir_all(&without_remote).unwrap();
        assert!(Command::new("git")
            .arg("-C")
            .arg(&without_remote)
            .args(["init", "--quiet"])
            .status()
            .unwrap()
            .success());
        assert_eq!(repository_id_for(&without_remote), None);
    }
}
