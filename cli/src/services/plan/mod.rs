pub mod command;
pub mod extract;
pub mod validate;

pub const NAME: &str = "plan";

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

#[cfg(test)]
struct CommandRun {
    exit_code: std::process::ExitCode,
    stdout: String,
    stderr: String,
}

#[cfg(test)]
fn run_plan_check(path: &str) -> CommandRun {
    let command = crate::services::parse::command_runtime::parse_runtime_command(
        ["sce", NAME, "check", path].map(String::from),
        &crate::services::command_registry::CommandRegistry::default(),
        None,
    )
    .expect("plan check should parse");
    let crate::services::command_registry::RuntimeCommand::Plan(command) = command else {
        panic!("expected plan command");
    };
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let exit_code = crate::services::app_support::render_run_outcome(
        crate::services::app_support::RunOutcome {
            result: command.execute(),
            logger: None::<crate::services::observability::traits::NoopLogger>,
            startup_diagnostic: None,
        },
        &mut stdout,
        &mut stderr,
    );

    CommandRun {
        exit_code,
        stdout: String::from_utf8(stdout).expect("stdout should be UTF-8"),
        stderr: String::from_utf8(stderr).expect("stderr should be UTF-8"),
    }
}

#[test]
fn command_valid_plans_exit_success() {
    let relative_dir = "../config/pkl/renderers/fixtures/plan-structure";
    for (fixture, expected) in [
        ("valid-structured-plan.md", command::STRUCTURED_PLAN_PASS),
        ("inspection-only-plan.md", command::STRUCTURED_PLAN_PASS),
        ("legacy-plan.md", command::LEGACY_PLAN_PASS),
    ] {
        let absolute = fixture_dir().join(fixture);
        for path in [
            format!("{relative_dir}/{fixture}"),
            absolute.display().to_string(),
        ] {
            let run = run_plan_check(&path);
            assert_eq!(run.exit_code, std::process::ExitCode::SUCCESS, "{path}");
            assert_eq!(run.stdout, format!("{expected}\n"), "{path}");
            assert!(run.stderr.is_empty(), "{path}: {}", run.stderr);
        }
    }
}

#[test]
fn command_invalid_plan_exits_validation() {
    let path = "../config/pkl/renderers/fixtures/plan-structure/duplicate-id.md";
    let run = run_plan_check(path);

    assert_eq!(run.exit_code, std::process::ExitCode::from(3));
    assert!(run.stdout.is_empty(), "{}", run.stdout);
    assert!(run.stderr.contains("SCE-ERR-VALIDATION"), "{}", run.stderr);
    assert_eq!(
        run.stderr
            .lines()
            .filter(|line| line.starts_with(&format!("{path}:")))
            .collect::<Vec<_>>(),
        vec![format!("{path}:29: `S1` is defined more than once")]
    );
}

#[test]
fn command_missing_file_is_runtime_failure() {
    let directory = tempfile::tempdir().expect("tempdir should be created");
    let path = directory
        .path()
        .join("missing-plan.md")
        .display()
        .to_string();
    let run = run_plan_check(&path);

    assert_eq!(run.exit_code, std::process::ExitCode::from(4));
    assert!(run.stdout.is_empty(), "{}", run.stdout);
    assert!(run.stderr.contains("SCE-ERR-RUNTIME"), "{}", run.stderr);
    assert!(run.stderr.contains(&path), "{}", run.stderr);
    assert!(!run.stderr.contains(&format!("{path}:")), "{}", run.stderr);
}

#[test]
fn command_does_not_execute_plan_commands() {
    let directory = tempfile::tempdir().expect("tempdir should be created");
    let marker = directory.path().join("executed");
    let touch = format!("`touch {}`", marker.display());
    let plan_path = directory.path().join("plan.md");
    std::fs::write(
        &plan_path,
        format!(
            "# Plan: no-execution\n\n## Acceptance criteria\n\n- [ ] AC1: Nothing runs.\n  - Validate: {touch}\n  - Scenarios: S1\n\n### Verification scenarios\n\n- S1: Commands stay inert\n  - Given: A plan.\n  - When: It is checked.\n  - Then: Nothing runs.\n  - Check: {touch}\n\n## Task stack\n\n- [ ] T01: `Do nothing` (status:todo)\n  - Task ID: T01\n  - Covers: AC1\n  - Verify: {touch}\n"
        ),
    )
    .expect("plan should be written");

    let run = run_plan_check(&plan_path.display().to_string());

    assert_eq!(
        run.exit_code,
        std::process::ExitCode::SUCCESS,
        "{}",
        run.stderr
    );
    assert_eq!(run.stdout, format!("{}\n", command::STRUCTURED_PLAN_PASS));
    assert!(!marker.exists());
}

#[test]
fn plan_check_appears_in_help() {
    assert!(crate::services::help::help_text().contains("sce plan"));
    let help = crate::cli_schema::render_help_for_path(&[NAME, "check"])
        .expect("plan check help should be renderable");
    assert!(help.contains("<PATH>"), "{help}");
}

#[test]
fn completion_scripts_include_plan_check() {
    use crate::services::completion::{render_completion, CompletionRequest, CompletionShell};

    for shell in [
        CompletionShell::Bash,
        CompletionShell::Zsh,
        CompletionShell::Fish,
    ] {
        let script = render_completion(CompletionRequest { shell });
        assert!(
            script.contains("plan") && script.contains("check"),
            "{shell:?}"
        );
    }
}
