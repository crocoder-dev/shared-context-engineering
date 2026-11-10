use std::fmt;

use serde_json::Value;

use crate::services::agent_trace::sce_config_schema_url;
use crate::services::version::PACKAGE_VERSION;

const SCHEMA_URL_PREFIX: &str = "https://sce.crocoder.dev/v";
const SCHEMA_URL_SUFFIX: &str = "/config.json";

pub(crate) const SCHEMA_DECLARATION_POINTER: &str = "/$schema";

const OLDER_SERIES_ISSUE_PREFIX: &str = "/$schema: declares SCE config schema version ";
const OLDER_SERIES_REMEDIATION: &str =
    "run 'sce setup' to upgrade the repository and global config";

const ISSUE_SEPARATOR: &str = " | ";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SchemaSeries {
    pub(crate) major: u64,
    pub(crate) minor: u64,
}

impl fmt::Display for SchemaSeries {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}", self.major, self.minor)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SchemaVersion {
    pub(crate) major: u64,
    pub(crate) minor: u64,
    pub(crate) patch: u64,
    pub(crate) prerelease: Option<String>,
    pub(crate) build: Option<String>,
}

impl SchemaVersion {
    /// Accepts exactly the semantic-version grammar used by the Pkl
    /// `seriesPattern`, so classification stays in parity with the embedded schema.
    pub(crate) fn parse(raw: &str) -> Option<Self> {
        let (rest, build) = match raw.split_once('+') {
            Some((rest, build)) => (rest, Some(parse_identifiers(build)?)),
            None => (raw, None),
        };
        let (core, prerelease) = match rest.split_once('-') {
            Some((core, prerelease)) => (core, Some(parse_identifiers(prerelease)?)),
            None => (rest, None),
        };

        let mut numbers = core.split('.');
        let major = parse_numeric(numbers.next()?)?;
        let minor = parse_numeric(numbers.next()?)?;
        let patch = parse_numeric(numbers.next()?)?;
        if numbers.next().is_some() {
            return None;
        }

        Some(Self {
            major,
            minor,
            patch,
            prerelease,
            build,
        })
    }

    pub(crate) fn series(&self) -> SchemaSeries {
        SchemaSeries {
            major: self.major,
            minor: self.minor,
        }
    }

    pub(crate) fn release(&self) -> String {
        format!("{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl fmt::Display for SchemaVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if let Some(prerelease) = &self.prerelease {
            write!(formatter, "-{prerelease}")?;
        }
        if let Some(build) = &self.build {
            write!(formatter, "+{build}")?;
        }
        Ok(())
    }
}

fn parse_numeric(raw: &str) -> Option<u64> {
    let valid = raw == "0"
        || (raw.starts_with(|first: char| ('1'..='9').contains(&first))
            && raw.bytes().all(|byte| byte.is_ascii_digit()));
    if valid {
        raw.parse().ok()
    } else {
        None
    }
}

fn parse_identifiers(raw: &str) -> Option<String> {
    let valid = raw.split('.').all(|identifier| {
        !identifier.is_empty()
            && identifier
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    });
    valid.then(|| raw.to_string())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SchemaDeclaration {
    Absent,
    CurrentSeries,
    OlderSeries(SchemaVersion),
    NewerSeries(SchemaVersion),
    Unrecognized,
}

pub(crate) fn current_schema_version() -> SchemaVersion {
    SchemaVersion::parse(PACKAGE_VERSION).expect("SCE package version should be a semantic version")
}

pub(crate) fn current_schema_series() -> SchemaSeries {
    current_schema_version().series()
}

pub(crate) fn classify_schema_declaration(
    value: Option<&Value>,
    current: SchemaSeries,
) -> SchemaDeclaration {
    let Some(value) = value else {
        return SchemaDeclaration::Absent;
    };
    let Some(version) = value
        .as_str()
        .and_then(|url| url.strip_prefix(SCHEMA_URL_PREFIX))
        .and_then(|rest| rest.strip_suffix(SCHEMA_URL_SUFFIX))
        .and_then(SchemaVersion::parse)
    else {
        return SchemaDeclaration::Unrecognized;
    };

    let declared = version.series();
    match (declared.major, declared.minor).cmp(&(current.major, current.minor)) {
        std::cmp::Ordering::Equal => SchemaDeclaration::CurrentSeries,
        std::cmp::Ordering::Less => SchemaDeclaration::OlderSeries(version),
        std::cmp::Ordering::Greater => SchemaDeclaration::NewerSeries(version),
    }
}

/// Returns the single validation issue reported for a rejected `$schema`
/// declaration, or `None` when the declaration itself is acceptable.
pub(crate) fn schema_declaration_issue(
    declaration: &SchemaDeclaration,
    current: &SchemaVersion,
) -> Option<String> {
    let current_url = sce_config_schema_url();
    let current_release = current.release();
    let current_series = current.series();
    match declaration {
        SchemaDeclaration::Absent | SchemaDeclaration::CurrentSeries => None,
        SchemaDeclaration::OlderSeries(version) => Some(format!(
            "{OLDER_SERIES_ISSUE_PREFIX}{}; this CLI uses {current_release} and accepts {current_series}.x; {OLDER_SERIES_REMEDIATION}, or set `$schema` to {current_url}",
            version.release()
        )),
        SchemaDeclaration::NewerSeries(version) => Some(format!(
            "{SCHEMA_DECLARATION_POINTER}: declares SCE config schema version {}, which was written by a newer SCE release; this CLI uses {current_release} and accepts {current_series}.x; upgrade the SCE CLI",
            version.release()
        )),
        SchemaDeclaration::Unrecognized => Some(format!(
            "{SCHEMA_DECLARATION_POINTER}: is not a canonical SCE schema URL; use {current_url}"
        )),
    }
}

/// Recognizes a config validation error whose only issue is an older-series
/// `$schema` declaration. Issues are sorted and joined with `" | "`, so any
/// additional issue leaves a separator after the older-series issue.
pub(crate) fn is_older_series_only_failure(error: &str) -> bool {
    let Some(start) = error.rfind(OLDER_SERIES_ISSUE_PREFIX) else {
        return false;
    };
    let issue = &error[start..];
    issue.contains(OLDER_SERIES_REMEDIATION)
        && !issue.contains(ISSUE_SEPARATOR)
        && !error[..start].contains(ISSUE_SEPARATOR)
}

#[cfg(test)]
mod schema_declaration_tests {
    use std::path::Path;

    use serde_json::{json, Value};

    use super::{
        classify_schema_declaration, current_schema_series, current_schema_version,
        is_older_series_only_failure, SchemaDeclaration, SchemaSeries, SchemaVersion,
    };
    use crate::services::config::schema::parse_file_config;
    use crate::services::config::ConfigPathSource;
    use crate::services::version::PACKAGE_VERSION;

    const INJECTED_SERIES: SchemaSeries = SchemaSeries { major: 0, minor: 5 };

    fn schema_url(version: &str) -> String {
        format!("https://sce.crocoder.dev/v{version}/config.json")
    }

    fn older_than_current() -> String {
        let SchemaSeries { major, minor } = current_schema_series();
        if minor > 0 {
            format!("{major}.{}.3", minor - 1)
        } else {
            format!("{}.9.0", major - 1)
        }
    }

    fn parse_error(value: &Value) -> Option<String> {
        parse_file_config(
            &value.to_string(),
            Path::new("/tmp/sce-config.json"),
            ConfigPathSource::Flag,
        )
        .err()
        .map(|error| error.to_string())
    }

    fn declaration_matrix() -> Vec<Value> {
        let SchemaSeries { major, minor } = current_schema_series();
        let mut versions = vec![
            PACKAGE_VERSION.to_string(),
            format!("{major}.{minor}.0"),
            format!("{major}.{minor}.7"),
            format!("{major}.{minor}.0-pre-alpha-v1"),
            format!("{major}.{minor}.0-pre-alpha-v14"),
            format!("{major}.{minor}.7+build.5"),
            format!("{major}.{minor}.12-rc.1+build.5"),
            format!("{major}.{minor}.0-rc..1"),
            format!("{major}.{minor}.0-"),
            format!("{major}.{minor}.0+"),
            format!("{major}.{minor}.0-rc_1"),
            format!("{major}.{minor}"),
            format!("{major}.{minor}.01"),
            format!("{major}.{minor}.x"),
            format!("{major}.{minor}.0.0"),
            format!("{major}.{}.0", minor + 1),
            format!("{}.{minor}.0", major + 1),
            format!("{}.0.0", major + 1),
            "0.3.3".to_string(),
            "0.1.0".to_string(),
            "1.4.0".to_string(),
            format!("0{major}.{minor}.0"),
        ];
        if minor > 0 {
            versions.push(format!("{major}.{}.3", minor - 1));
        }

        let mut values: Vec<Value> = versions
            .iter()
            .map(|version| json!(schema_url(version)))
            .collect();
        values.extend([
            json!("https://example.com/s.json"),
            json!("https://sce.crocoder.dev/config.json"),
            json!(format!(
                "https://sce.crocoder.dev/v{major}.{minor}/config.json"
            )),
            json!(format!(
                "http://sce.crocoder.dev/v{major}.{minor}.0/config.json"
            )),
            json!(format!(
                "https://sce.crocoder.dev/v{major}.{minor}.0/config.json.bak"
            )),
            json!(format!(
                " https://sce.crocoder.dev/v{major}.{minor}.0/config.json"
            )),
            json!(1),
            json!(null),
            json!({ "url": schema_url(PACKAGE_VERSION) }),
        ]);
        values
    }

    #[test]
    fn schema_declaration_classifier_matches_embedded_schema() {
        let current = current_schema_series();
        for value in declaration_matrix() {
            let accepted = parse_error(&json!({ "$schema": value.clone() })).is_none();
            let classified = classify_schema_declaration(Some(&value), current);
            assert_eq!(
                accepted,
                classified == SchemaDeclaration::CurrentSeries,
                "parity mismatch for {value}: classified {classified:?}"
            );
        }
    }

    #[test]
    fn schema_declaration_absent_is_accepted() {
        assert_eq!(
            classify_schema_declaration(None, current_schema_series()),
            SchemaDeclaration::Absent
        );
        assert!(parse_error(&json!({})).is_none());
    }

    #[test]
    fn schema_declaration_classifies_against_injected_series() {
        let older = ["0.4.0-pre-alpha-v1", "0.4.7", "0.3.3"];
        for version in older {
            let classified =
                classify_schema_declaration(Some(&json!(schema_url(version))), INJECTED_SERIES);
            assert_eq!(
                classified,
                SchemaDeclaration::OlderSeries(SchemaVersion::parse(version).unwrap()),
                "{version}"
            );
        }

        assert_eq!(
            classify_schema_declaration(Some(&json!(schema_url("1.4.0"))), INJECTED_SERIES),
            SchemaDeclaration::NewerSeries(SchemaVersion::parse("1.4.0").unwrap())
        );
        assert_eq!(
            classify_schema_declaration(Some(&json!(schema_url("0.5.3+b.1"))), INJECTED_SERIES),
            SchemaDeclaration::CurrentSeries
        );
        assert_eq!(
            classify_schema_declaration(Some(&json!(schema_url("0.5"))), INJECTED_SERIES),
            SchemaDeclaration::Unrecognized
        );
    }

    #[test]
    fn schema_declaration_version_round_trips_prerelease_and_build() {
        let version = SchemaVersion::parse("0.4.0-pre-alpha-v14+build.5").unwrap();
        assert_eq!(version.prerelease.as_deref(), Some("pre-alpha-v14"));
        assert_eq!(version.build.as_deref(), Some("build.5"));
        assert_eq!(version.to_string(), "0.4.0-pre-alpha-v14+build.5");
    }

    #[test]
    fn schema_declaration_older_series_reports_setup_remediation() {
        let current = current_schema_version();
        let older = older_than_current();
        let error = parse_error(&json!({ "$schema": schema_url(&older) })).unwrap();

        let declared = SchemaVersion::parse(&older).unwrap().release();
        assert!(
            error.contains(&format!(
                "/$schema: declares SCE config schema version {declared}; this CLI uses {} and accepts {}.x; run 'sce setup'",
                current.release(),
                current.series()
            )),
            "{error}"
        );
        assert!(error.contains(&schema_url(PACKAGE_VERSION)), "{error}");
        assert!(is_older_series_only_failure(&error), "{error}");
    }

    #[test]
    fn schema_declaration_newer_series_reports_cli_upgrade() {
        let current = current_schema_series();
        let newer = format!("{}.{}.0", current.major, current.minor + 1);
        let error = parse_error(&json!({ "$schema": schema_url(&newer) })).unwrap();

        assert!(
            error.contains(&format!("declares SCE config schema version {newer}, which was written by a newer SCE release; this CLI uses")),
            "{error}"
        );
        assert!(error.contains("upgrade the SCE CLI"), "{error}");
        assert!(!is_older_series_only_failure(&error), "{error}");
    }

    #[test]
    fn schema_declaration_unrecognized_reports_current_url() {
        for value in [json!("https://example.com/s.json"), json!(1)] {
            let error = parse_error(&json!({ "$schema": value })).unwrap();
            assert!(
                error.contains(&format!(
                    "/$schema: is not a canonical SCE schema URL; use {}",
                    schema_url(PACKAGE_VERSION)
                )),
                "{error}"
            );
            assert_eq!(error.matches("/$schema").count(), 1, "{error}");
            assert!(!is_older_series_only_failure(&error), "{error}");
        }
    }

    #[test]
    fn schema_declaration_issue_is_reported_alongside_other_field_errors() {
        let error = parse_error(&json!({
            "$schema": schema_url(&older_than_current()),
            "log_level": "loud",
        }))
        .unwrap();

        assert!(error.contains("/$schema: "), "{error}");
        assert!(error.contains("/log_level: "), "{error}");
        assert!(error.contains(" | "), "{error}");
        assert!(!is_older_series_only_failure(&error), "{error}");
    }
}
