pub mod command;
pub mod extract;
pub mod init;
pub mod validate;

pub const NAME: &str = "plan";

pub(crate) const SCAFFOLD: &str = include_str!(concat!(
    env!("OUT_DIR"),
    "/pkl-generated/config/plan-init-scaffold.md"
));

pub(crate) const SCAFFOLD_PLAN_NAME_TOKEN: &str = "{plan-name}";

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
        let mut expected: Vec<&str> = columns.collect();
        let report = validate_fixture(fixture);
        listed.push(fixture.to_string());
        let mut actual: Vec<&str> = report
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.message.as_str())
            .collect();
        expected.sort_unstable();
        actual.sort_unstable();

        match verdict {
            "pass" => assert!(
                expected.is_empty(),
                "{fixture}: pass rows take no diagnostics"
            ),
            "fail" => assert!(
                !expected.is_empty(),
                "{fixture}: fail rows need diagnostics"
            ),
            other => panic!("{fixture}: unknown verdict `{other}`"),
        }
        assert_eq!(actual, expected, "{fixture}: diagnostics differ");
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
        diagnostic_lines(&validate_fixture("misplaced-definition.md")),
        vec![
            (
                Some(20),
                "`Scenarios:` on `AC1` references undefined `S1`; it appears only outside `### Verification scenarios`"
            ),
            (
                Some(21),
                "`S1` is defined under `## Acceptance criteria`; it belongs under `### Verification scenarios`"
            ),
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

#[test]
fn misplaced_definition_ignores_prose_and_fenced_mentions() {
    let source = "# Plan\n\n## Acceptance criteria\n\n- [ ] AC1: Works.\n  - Validate: `true`\n  - Scenarios: S1\n  - S2: nested prose\n\n### Verification scenarios\n\n- S1: Works\n  - Given: A.\n  - When: B.\n  - Then: C.\n  - Check: `true`\n\n```markdown\n- T02: fenced example\n```\n\n## Task stack\n\n- [ ] T01: `Work` (status:todo)\n  - Covers: AC1\n\n## Open questions\n\n- D1: incidental prose\n";

    assert!(diagnostic_lines(&validate_source(source)).is_empty());
    assert_eq!(
        diagnostic_lines(&validate_source(&source.replace("  - S2: nested prose\n", "- S2: misplaced\n"))),
        vec![
            (
                Some(8),
                "`S2` is defined under `## Acceptance criteria`; it belongs under `### Verification scenarios`"
            ),
        ]
    );
}

#[test]
fn scaffold_untouched_fails_check() {
    assert_eq!(SCAFFOLD.matches(SCAFFOLD_PLAN_NAME_TOKEN).count(), 1);
    let source = SCAFFOLD.replacen(SCAFFOLD_PLAN_NAME_TOKEN, "untouched", 1);
    assert!(source.starts_with("# Plan: untouched\n"), "{source}");

    let plan = extract::extract(&source).expect("scaffold should parse");
    assert_eq!(plan.entries_of(extract::PlanEntryKind::Decision).count(), 0);
    assert_eq!(plan.entries_of(extract::PlanEntryKind::Scenario).count(), 0);

    let report = validate::validate(&plan);
    let mut actual: Vec<&str> = report
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.message.as_str())
        .collect();
    let mut expected = vec![
        "`Scenarios:` on `AC1` is not a comma-separated ID list",
        "`AC1` is not covered by any task's `Covers:`",
        "`Covers:` on `T01` is not a comma-separated ID list",
    ];
    actual.sort_unstable();
    expected.sort_unstable();
    assert_eq!(actual, expected);
}

#[cfg(test)]
struct CommandRun {
    exit_code: std::process::ExitCode,
    stdout: String,
    stderr: String,
}

#[cfg(test)]
fn run_plan_args(args: &[&str], start: &std::path::Path) -> CommandRun {
    let mut stderr = Vec::new();
    let result = crate::services::parse::command_runtime::parse_runtime_command(
        std::iter::once("sce")
            .chain(args.iter().copied())
            .map(String::from),
        &crate::services::command_registry::CommandRegistry::default(),
        None,
    )
    .and_then(|command| {
        let crate::services::command_registry::RuntimeCommand::Plan(command) = command else {
            panic!("expected plan command");
        };
        command.execute_with(|| Ok(start.to_path_buf()), &mut stderr)
    });
    render_plan_result(result, stderr)
}

#[cfg(test)]
fn render_plan_result(
    result: Result<String, crate::services::error::CliError>,
    mut stderr: Vec<u8>,
) -> CommandRun {
    let mut stdout = Vec::new();
    let exit_code = crate::services::app_support::render_run_outcome(
        crate::services::app_support::RunOutcome {
            result,
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

#[cfg(test)]
fn run_plan_check(path: &str) -> CommandRun {
    run_plan_args(&[NAME, "check", path], std::path::Path::new("."))
}

#[test]
fn command_valid_plans_exit_success() {
    let relative_dir = "../config/pkl/renderers/fixtures/plan-structure";
    for (fixture, expected) in [
        ("valid-structured-plan.md", command::STRUCTURED_PLAN_PASS),
        ("inspection-only-plan.md", command::STRUCTURED_PLAN_PASS),
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
fn command_unstructured_plan_exits_validation() {
    let path = "../config/pkl/renderers/fixtures/plan-structure/unstructured-plan.md";
    let run = run_plan_check(path);

    assert_eq!(run.exit_code, std::process::ExitCode::from(3));
    assert!(run.stdout.is_empty(), "{}", run.stdout);
    assert!(
        run.stderr
            .contains(&format!("{path}:59: `T01` is missing `Covers:`")),
        "{}",
        run.stderr
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

#[cfg(test)]
const SCAFFOLD_CHECK_DEFECTS: [&str; 3] = [
    "`AC1` is not covered by any task's `Covers:`",
    "`Covers:` on `T01` is not a comma-separated ID list",
    "`Scenarios:` on `AC1` is not a comma-separated ID list",
];

#[cfg(test)]
fn git_init(root: &std::path::Path) {
    let output = std::process::Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(root)
        .output()
        .expect("git should run");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(test)]
fn temp_repository() -> (tempfile::TempDir, std::path::PathBuf) {
    let directory = tempfile::tempdir().expect("tempdir should be created");
    let root = directory
        .path()
        .canonicalize()
        .expect("tempdir should canonicalize");
    git_init(&root);
    (directory, root)
}

#[cfg(test)]
fn tree_listing(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    fn walk(root: &std::path::Path, dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("directory should be readable") {
            let path = entry.expect("entry should be readable").path();
            if path.file_name().is_some_and(|name| name == ".git") {
                continue;
            }
            out.push(
                path.strip_prefix(root)
                    .expect("path under root")
                    .to_path_buf(),
            );
            if path.symlink_metadata().expect("metadata").is_dir() {
                walk(root, &path, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort();
    out
}

#[cfg(test)]
fn sorted_entries(dir: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .expect("directory should be readable")
        .map(|entry| {
            entry
                .expect("entry should be readable")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

#[cfg(test)]
fn expected_init_stdout(root: &std::path::Path, name: &str) -> String {
    let absolute = format!("{}/context/plans/{name}.md", root.display());
    format!(
        "Created plan: context/plans/{name}.md\nNext: complete the plan and run\nsce plan check '{}'\n",
        absolute.replace('\'', "'\\''")
    )
}

#[cfg(test)]
fn check_defects(stderr: &str, path: &str) -> Vec<String> {
    let prefix = format!("{path}:");
    let mut defects: Vec<String> = stderr
        .lines()
        .filter_map(|line| line.strip_prefix(&prefix))
        .map(|rest| {
            rest.split_once(": ")
                .map_or(rest, |(_, message)| message)
                .to_string()
        })
        .collect();
    defects.sort();
    defects
}

#[test]
fn init_creates_scaffold_at_repository_root() {
    let (_directory, root) = temp_repository();
    let run = run_plan_args(&[NAME, "init", "example"], &root);

    assert_eq!(
        run.exit_code,
        std::process::ExitCode::SUCCESS,
        "{}",
        run.stderr
    );
    assert_eq!(run.stdout, expected_init_stdout(&root, "example"));
    assert!(run.stderr.is_empty(), "{}", run.stderr);

    let plans = root.join("context/plans");
    assert_eq!(
        std::fs::read_to_string(plans.join("example.md")).expect("plan should exist"),
        SCAFFOLD.replacen(SCAFFOLD_PLAN_NAME_TOKEN, "example", 1)
    );
    assert!(std::fs::read_to_string(plans.join("example.md"))
        .expect("plan should exist")
        .starts_with("# Plan: example\n"));
    assert_eq!(sorted_entries(&plans), vec!["example.md"]);

    let status = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(&root)
        .output()
        .expect("git status should run");
    assert_eq!(String::from_utf8_lossy(&status.stdout), "?? context/\n");
}

#[test]
fn init_from_nested_directory_targets_repository_root() {
    let directory = tempfile::tempdir().expect("tempdir should be created");
    let root = directory
        .path()
        .canonicalize()
        .expect("tempdir should canonicalize")
        .join("repo with 'quote'");
    std::fs::create_dir(&root).expect("root should be created");
    git_init(&root);
    let nested = root.join("a/b");
    std::fs::create_dir_all(&nested).expect("nested directory should be created");

    let run = run_plan_args(&[NAME, "init", "nested-plan"], &nested);

    assert_eq!(
        run.exit_code,
        std::process::ExitCode::SUCCESS,
        "{}",
        run.stderr
    );
    assert_eq!(run.stdout, expected_init_stdout(&root, "nested-plan"));
    let created = root.join("context/plans/nested-plan.md");
    assert!(created.is_file());
    assert!(sorted_entries(&nested).is_empty());

    let lines: Vec<&str> = run.stdout.lines().collect();
    assert_eq!(lines[0], "Created plan: context/plans/nested-plan.md");
    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("sce() {{ printf '%s\\0' \"$@\"; }}\n{}", lines[2]))
        .current_dir(&nested)
        .output()
        .expect("sh should run");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let decoded: Vec<String> = String::from_utf8(output.stdout)
        .expect("decoded arguments should be UTF-8")
        .split_terminator('\0')
        .map(String::from)
        .collect();
    let created_path = created.display().to_string();
    assert_eq!(
        decoded,
        vec![NAME.to_string(), "check".to_string(), created_path.clone()]
    );

    let arguments: Vec<&str> = decoded.iter().map(String::as_str).collect();
    let check = run_plan_args(&arguments, &nested);
    assert_eq!(
        check.exit_code,
        std::process::ExitCode::from(3),
        "{}",
        check.stderr
    );
    assert!(
        check.stderr.contains("SCE-ERR-VALIDATION"),
        "{}",
        check.stderr
    );
    assert!(
        !check.stderr.contains("Failed to read plan file"),
        "{}",
        check.stderr
    );
    assert_eq!(
        check_defects(&check.stderr, &created_path),
        SCAFFOLD_CHECK_DEFECTS
    );
}

#[test]
fn init_never_overwrites_existing_entry() {
    let (_directory, root) = temp_repository();
    let plans = root.join("context/plans");
    std::fs::create_dir_all(&plans).expect("plans directory should be created");
    std::fs::write(plans.join("taken.md"), b"known bytes\n").expect("taken should be written");
    std::os::unix::fs::symlink("missing-target.md", plans.join("linked.md"))
        .expect("symlink should be created");

    for name in ["taken", "linked"] {
        let run = run_plan_args(&[NAME, "init", name], &root);
        assert_eq!(
            run.exit_code,
            std::process::ExitCode::from(3),
            "{name}: {}",
            run.stderr
        );
        assert!(run.stdout.is_empty(), "{name}: {}", run.stdout);
        assert!(
            run.stderr.contains("SCE-ERR-VALIDATION"),
            "{name}: {}",
            run.stderr
        );
        assert!(
            run.stderr
                .contains(&format!("Plan already exists: context/plans/{name}.md")),
            "{name}: {}",
            run.stderr
        );
    }

    assert_eq!(
        std::fs::read(plans.join("taken.md")).expect("taken should be readable"),
        b"known bytes\n"
    );
    assert_eq!(
        std::fs::read_link(plans.join("linked.md")).expect("linked should stay a symlink"),
        std::path::PathBuf::from("missing-target.md")
    );
    assert_eq!(sorted_entries(&plans), vec!["linked.md", "taken.md"]);
}

#[cfg(test)]
fn run_init_with(
    root: &std::path::Path,
    name: &str,
    link: impl FnOnce(&std::path::Path, &std::path::Path) -> std::io::Result<()>,
    remove: impl FnOnce(&std::path::Path) -> std::io::Result<()>,
) -> CommandRun {
    let mut stderr = Vec::new();
    let plan_name = init::PlanName::parse(name).expect("name should be valid");
    let result = init::create_with(&plan_name, root, &mut stderr, link, remove);
    render_plan_result(result, stderr)
}

#[cfg(test)]
fn injected(message: &str) -> std::io::Error {
    std::io::Error::other(message.to_string())
}

#[test]
fn init_cleanup_failure_after_publication_warns_and_succeeds() {
    let (_directory, root) = temp_repository();
    let plans = root.join("context/plans");
    let temporary = init::temporary_path(&plans, "published.md");

    let run = run_init_with(
        &root,
        "published",
        |temporary, destination| std::fs::hard_link(temporary, destination),
        |_| Err(injected("injected cleanup failure")),
    );

    assert_eq!(
        run.exit_code,
        std::process::ExitCode::SUCCESS,
        "{}",
        run.stderr
    );
    assert_eq!(run.stdout, expected_init_stdout(&root, "published"));
    assert_eq!(
        run.stderr,
        format!(
            "Warning: Created plan context/plans/published.md, but failed to remove temporary file '{0}': injected cleanup failure. Try: remove '{0}' manually; the plan was created and does not need to be initialized again.\n",
            temporary.display()
        )
    );
    assert_eq!(
        std::fs::read_to_string(plans.join("published.md")).expect("plan should exist"),
        SCAFFOLD.replacen(SCAFFOLD_PLAN_NAME_TOKEN, "published", 1)
    );
    assert!(temporary.is_file());
}

#[test]
fn init_publication_failure_removes_temporary_and_keeps_error() {
    let (_directory, root) = temp_repository();
    let plans = root.join("context/plans");

    let run = run_init_with(
        &root,
        "unpublished",
        |_, _| Err(injected("injected link failure")),
        |temporary| std::fs::remove_file(temporary),
    );

    assert_eq!(
        run.exit_code,
        std::process::ExitCode::from(4),
        "{}",
        run.stderr
    );
    assert!(run.stdout.is_empty(), "{}", run.stdout);
    assert!(run.stderr.contains("SCE-ERR-RUNTIME"), "{}", run.stderr);
    assert!(run.stderr.contains("Failed to publish"), "{}", run.stderr);
    assert!(
        run.stderr.contains("injected link failure"),
        "{}",
        run.stderr
    );
    assert!(
        !run.stderr.contains("Cleanup also failed"),
        "{}",
        run.stderr
    );
    assert!(sorted_entries(&plans).is_empty());
}

#[test]
fn init_publication_and_cleanup_failure_keeps_original_error_and_destination() {
    let (_directory, root) = temp_repository();
    let plans = root.join("context/plans");
    std::fs::create_dir_all(&plans).expect("plans directory should be created");
    std::fs::write(plans.join("taken.md"), b"known bytes\n").expect("taken should be written");
    let temporary = init::temporary_path(&plans, "taken.md");

    let run = run_init_with(
        &root,
        "taken",
        |temporary, destination| std::fs::hard_link(temporary, destination),
        |_| Err(injected("injected cleanup failure")),
    );

    assert_eq!(
        run.exit_code,
        std::process::ExitCode::from(3),
        "{}",
        run.stderr
    );
    assert!(run.stdout.is_empty(), "{}", run.stdout);
    assert!(run.stderr.contains("SCE-ERR-VALIDATION"), "{}", run.stderr);
    assert!(
        run.stderr
            .contains("Plan already exists: context/plans/taken.md"),
        "{}",
        run.stderr
    );
    assert!(
        run.stderr.contains(&format!(
            "Cleanup also failed: could not remove temporary file '{}': injected cleanup failure",
            temporary.display()
        )),
        "{}",
        run.stderr
    );
    assert!(!run.stderr.contains("Warning:"), "{}", run.stderr);
    assert_eq!(
        std::fs::read(plans.join("taken.md")).expect("taken should be readable"),
        b"known bytes\n"
    );
}

#[test]
fn init_rejects_unsafe_names_without_writing() {
    let (_directory, root) = temp_repository();
    let before = tree_listing(&root);
    let too_long = "a".repeat(init::PLAN_NAME_MAX_BYTES + 1);

    for name in [
        "../escape",
        "a/b",
        "Upper",
        "trailing-",
        "double--hyphen",
        "plan.md",
        "",
        too_long.as_str(),
    ] {
        let run = run_plan_args(&[NAME, "init", name], &root);
        assert_eq!(
            run.exit_code,
            std::process::ExitCode::from(3),
            "{name:?}: {}",
            run.stderr
        );
        assert!(
            run.stderr.contains("SCE-ERR-VALIDATION"),
            "{name:?}: {}",
            run.stderr
        );
        assert!(
            run.stderr.contains("Invalid plan name"),
            "{name:?}: {}",
            run.stderr
        );
    }
    let run = run_plan_args(&[NAME, "init", "--", "-leading"], &root);
    assert_eq!(
        run.exit_code,
        std::process::ExitCode::from(3),
        "{}",
        run.stderr
    );
    assert!(
        run.stderr.contains("Invalid plan name '-leading'"),
        "{}",
        run.stderr
    );

    for arguments in [
        vec![NAME, "init", "-leading"],
        vec![NAME, "init", "--force", "example"],
        vec![NAME, "init", "one", "two"],
    ] {
        let run = run_plan_args(&arguments, &root);
        assert_eq!(
            run.exit_code,
            std::process::ExitCode::from(2),
            "{arguments:?}: {}",
            run.stderr
        );
        assert!(
            run.stderr.contains("SCE-ERR-PARSE"),
            "{arguments:?}: {}",
            run.stderr
        );
    }

    let run = run_plan_args(&[NAME, "init"], &root);
    assert_eq!(
        run.exit_code,
        std::process::ExitCode::from(3),
        "{}",
        run.stderr
    );
    assert!(run.stderr.contains("SCE-ERR-VALIDATION"), "{}", run.stderr);
    assert!(run.stderr.contains("<NAME>"), "{}", run.stderr);

    assert!(!root.join("context").exists());
    assert_eq!(tree_listing(&root), before);
}

#[test]
fn init_outside_repository_is_runtime_failure() {
    let directory = tempfile::tempdir().expect("tempdir should be created");
    let start = directory.path();
    let run = run_plan_args(&[NAME, "init", "example"], start);

    assert_eq!(
        run.exit_code,
        std::process::ExitCode::from(4),
        "{}",
        run.stderr
    );
    assert!(run.stdout.is_empty(), "{}", run.stdout);
    assert!(run.stderr.contains("SCE-ERR-RUNTIME"), "{}", run.stderr);
    assert!(tree_listing(start).is_empty());
    assert!(!start.join("context").exists());
}

#[test]
fn scaffold_authored_passes_check() {
    let (_directory, root) = temp_repository();
    let run = run_plan_args(&[NAME, "init", "authored"], &root);
    assert_eq!(
        run.exit_code,
        std::process::ExitCode::SUCCESS,
        "{}",
        run.stderr
    );

    let path = root.join("context/plans/authored.md");
    let source = std::fs::read_to_string(&path).expect("plan should exist");
    assert_eq!(source.matches("Scenarios: TODO").count(), 1);
    assert_eq!(source.matches("Covers: TODO").count(), 1);
    let authored = source
        .replacen("Scenarios: TODO", "Scenarios: none (inspection)", 1)
        .replacen("Covers: TODO", "Covers: AC1", 1);
    std::fs::write(&path, &authored).expect("plan should be written");

    let check = run_plan_check(&path.display().to_string());
    assert_eq!(
        check.exit_code,
        std::process::ExitCode::SUCCESS,
        "{}",
        check.stderr
    );
    assert_eq!(check.stdout, format!("{}\n", command::STRUCTURED_PLAN_PASS));
    assert!(authored.contains("TODO"));
}

#[test]
fn plan_init_appears_in_help() {
    let root_help = strip_ansi(&crate::services::help::help_text());
    assert!(root_help.contains("sce plan init"), "{root_help}");
    let plan_help = strip_ansi(
        &crate::cli_schema::render_help_for_path(&[NAME]).expect("plan help should be renderable"),
    );
    assert!(plan_help.contains("init"), "{plan_help}");
    let help = strip_ansi(
        &crate::cli_schema::render_help_for_path(&[NAME, "init"])
            .expect("plan init help should be renderable"),
    );
    assert!(help.contains("<NAME>"), "{help}");
}

#[cfg(test)]
fn strip_ansi(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(character) = chars.next() {
        if character != '\x1b' {
            output.push(character);
            continue;
        }
        match chars.next() {
            Some('[') => {
                for next in chars.by_ref() {
                    if ('@'..='~').contains(&next) {
                        break;
                    }
                }
            }
            Some(']') => {
                while let Some(next) = chars.next() {
                    if next == '\x07' {
                        break;
                    }
                    if next == '\x1b' && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    output
}

#[test]
fn completion_scripts_include_plan_init() {
    use crate::services::completion::{render_completion, CompletionRequest, CompletionShell};

    for shell in [
        CompletionShell::Bash,
        CompletionShell::Zsh,
        CompletionShell::Fish,
    ] {
        let script = render_completion(CompletionRequest { shell });
        assert!(
            script.contains("plan") && script.contains("init"),
            "{shell:?}"
        );
    }
}
