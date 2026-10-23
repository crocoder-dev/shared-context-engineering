pub mod extract;
pub mod validate;

#[cfg(test)]
fn fixture_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../config/pkl/renderers/fixtures/plan-structure")
}

#[cfg(test)]
fn validate_source(source: &str) -> validate::PlanReport {
    validate::validate(&extract::extract(source).expect("plan should parse"))
}

#[cfg(test)]
fn validate_fixture(name: &str) -> validate::PlanReport {
    let source =
        std::fs::read_to_string(fixture_dir().join(name)).expect("fixture should be readable");
    validate_source(&source)
}

#[cfg(test)]
fn diagnostic_lines(report: &validate::PlanReport) -> Vec<(Option<usize>, &str)> {
    report
        .diagnostics
        .iter()
        .map(|diagnostic| (diagnostic.line, diagnostic.message.as_str()))
        .collect()
}

#[test]
fn fixture_expectations_match_manifest() {
    let manifest = std::fs::read_to_string(fixture_dir().join("expectations.tsv"))
        .expect("manifest should be readable");
    let mut listed = Vec::new();

    for row in manifest
        .lines()
        .filter(|row| !row.trim().is_empty() && !row.starts_with('#'))
    {
        let mut columns = row.split('\t');
        let fixture = columns.next().expect("row should name a fixture");
        let verdict = columns.next().expect("row should carry a verdict");
        let expected: Vec<&str> = columns.collect();
        let report = validate_fixture(fixture);
        listed.push(fixture.to_string());

        match verdict {
            "structured" | "legacy" => {
                assert!(
                    expected.is_empty(),
                    "{fixture}: pass rows take no substrings"
                );
                assert_eq!(
                    report.structured,
                    verdict == "structured",
                    "{fixture}: wrong classification"
                );
                assert!(
                    report.diagnostics.is_empty(),
                    "{fixture}: unexpected diagnostics {:?}",
                    report.diagnostics
                );
            }
            "fail" => {
                assert!(!expected.is_empty(), "{fixture}: fail rows need substrings");
                for substring in expected {
                    assert!(
                        report
                            .diagnostics
                            .iter()
                            .any(|diagnostic| diagnostic.message.contains(substring)),
                        "{fixture}: missing `{substring}` in {:?}",
                        report.diagnostics
                    );
                }
            }
            other => panic!("{fixture}: unknown verdict `{other}`"),
        }
    }

    let mut fixtures: Vec<String> = std::fs::read_dir(fixture_dir())
        .expect("fixture directory should be readable")
        .map(|entry| {
            entry
                .expect("fixture entry should be readable")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter(|name| {
            std::path::Path::new(name)
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
        })
        .collect();
    fixtures.sort();
    listed.sort();
    assert_eq!(listed, fixtures, "manifest must list every fixture once");
}

#[test]
fn diagnostic_lines_match_source_positions() {
    assert_eq!(
        diagnostic_lines(&validate_fixture("misplaced-field.md")),
        vec![
            (
                Some(14),
                "`Check:` on `D1` is misplaced; it belongs to a verification scenario"
            ),
            (Some(18), "`AC1` is not covered by any task's `Covers:`"),
            (
                Some(22),
                "`Covers:` on `AC1` is misplaced; it belongs to a task"
            ),
            (Some(26), "`S1` is missing `Check:`"),
            (Some(37), "`T01` is missing `Covers:`"),
        ]
    );
    assert_eq!(
        diagnostic_lines(&validate_fixture("duplicate-id.md")),
        vec![(Some(29), "`S1` is defined more than once")]
    );
    assert_eq!(
        diagnostic_lines(&validate_fixture("task-empty-covers.md")),
        vec![(
            Some(47),
            "`Covers:` on `T02` is not a comma-separated ID list"
        )]
    );
    assert_eq!(
        diagnostic_lines(&validate_source(
            "# Plan\n\n## Acceptance criteria\n\n### Verification scenarios\n"
        )),
        vec![(None, "heading `## Task stack` is missing")]
    );
}
