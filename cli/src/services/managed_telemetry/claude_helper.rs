use std::fs;
use std::path::Path;

use super::{
    MANAGED_OTLP_ENDPOINT, MANAGED_OTLP_LOGS_ENDPOINT, MANAGED_OTLP_METRICS_ENDPOINT,
    MANAGED_OTLP_TRACES_ENDPOINT,
};
use crate::services::setup::config_merge::CLAUDE_SCE_HOOK_MARKER;

pub const CLAUDE_PROJECT_DIR_ENV: &str = "CLAUDE_PROJECT_DIR";
const BASE_ENDPOINT_ENV: &str = "OTEL_EXPORTER_OTLP_ENDPOINT";
const SIGNAL_ENDPOINT_ENVS: [(&str, &str); 3] = [
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
];
const GIT_DIR_NAME: &str = ".git";
const CLAUDE_DIR_NAME: &str = ".claude";
const CLAUDE_SETTINGS_FILE_NAME: &str = "settings.json";

fn normalized(value: &str) -> &str {
    value.trim().trim_end_matches('/')
}

pub fn endpoints_are_pinned<F>(lookup: F) -> bool
where
    F: Fn(&str) -> Option<String>,
{
    let base_matches =
        lookup(BASE_ENDPOINT_ENV).is_some_and(|value| normalized(&value) == MANAGED_OTLP_ENDPOINT);
    base_matches
        && SIGNAL_ENDPOINT_ENVS
            .iter()
            .all(|(key, expected)| lookup(key).is_none_or(|value| normalized(&value) == *expected))
}

pub fn repository_opted_in(project_dir: &Path) -> bool {
    let Some(root) = project_dir
        .ancestors()
        .find(|dir| dir.join(GIT_DIR_NAME).exists())
    else {
        return false;
    };
    fs::read_to_string(root.join(CLAUDE_DIR_NAME).join(CLAUDE_SETTINGS_FILE_NAME))
        .is_ok_and(|settings| settings.contains(CLAUDE_SCE_HOOK_MARKER))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn lookup_from(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map = pairs
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect::<HashMap<_, _>>();
        move |key| map.get(key).cloned()
    }

    #[test]
    fn helper_requires_managed_base_endpoint_and_matching_signal_overrides() {
        assert!(endpoints_are_pinned(lookup_from(&[(
            BASE_ENDPOINT_ENV,
            "https://otel.crocoderlab.dev/"
        )])));
        assert!(endpoints_are_pinned(lookup_from(&[
            (BASE_ENDPOINT_ENV, MANAGED_OTLP_ENDPOINT),
            (
                "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
                MANAGED_OTLP_TRACES_ENDPOINT
            ),
        ])));
        for rejected in [
            lookup_from(&[]),
            lookup_from(&[(BASE_ENDPOINT_ENV, "http://localhost:4318")]),
            lookup_from(&[
                (BASE_ENDPOINT_ENV, MANAGED_OTLP_ENDPOINT),
                (
                    "OTEL_EXPORTER_OTLP_LOGS_ENDPOINT",
                    "https://other.example/v1/logs",
                ),
            ]),
            lookup_from(&[
                (BASE_ENDPOINT_ENV, MANAGED_OTLP_ENDPOINT),
                ("OTEL_EXPORTER_OTLP_METRICS_ENDPOINT", ""),
            ]),
        ] {
            assert!(!endpoints_are_pinned(rejected));
        }
    }

    #[test]
    fn helper_attributes_only_repositories_with_sce_claude_integration() {
        let dir = tempfile::tempdir().unwrap();
        let opted_in = dir.path().join("opted-in");
        let nested = opted_in.join("src").join("module");
        fs::create_dir_all(&nested).unwrap();
        fs::create_dir_all(opted_in.join(".git")).unwrap();
        fs::create_dir_all(opted_in.join(".claude")).unwrap();
        fs::write(
            opted_in.join(".claude").join("settings.json"),
            format!(r#"{{"hooks":{{"Stop":[{{"command":"bash {CLAUDE_SCE_HOOK_MARKER}"}}]}}}}"#),
        )
        .unwrap();

        let unrelated = dir.path().join("unrelated");
        fs::create_dir_all(unrelated.join(".git")).unwrap();
        fs::create_dir_all(unrelated.join(".claude")).unwrap();
        fs::write(unrelated.join(".claude").join("settings.json"), "{}").unwrap();

        assert!(repository_opted_in(&opted_in));
        assert!(repository_opted_in(&nested));
        assert!(!repository_opted_in(&unrelated));
        assert!(!repository_opted_in(&dir.path().join("not-a-repo")));
    }
}
