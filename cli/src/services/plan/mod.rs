pub mod command;
pub mod extract;
pub mod init;
pub mod pr_description;
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
        diagnostic_lines(&validate_fixture("review-fields-missing-pair.md")),
        vec![
            (Some(9), "`D1` has `Review:` without `Review question:`"),
            (Some(15), "`D2` has `Review question:` without `Review:`"),
        ]
    );
    assert_eq!(
        diagnostic_lines(&validate_fixture("review-fields-malformed.md")),
        vec![
            (Some(14), "`Review:` on `D1` must be `required`"),
            (Some(22), "`Review question:` on `D2` is empty"),
            (Some(29), "`Review question:` on `D3` must be a single line"),
            (Some(36), "`Review:` on `D4` must be `required`"),
        ]
    );
    assert_eq!(
        diagnostic_lines(&validate_fixture("review-fields-duplicate.md")),
        vec![
            (Some(16), "`Review:` appears more than once on `D1`"),
            (
                Some(24),
                "`Review question:` appears more than once on `D2`"
            ),
        ]
    );
    assert_eq!(
        diagnostic_lines(&validate_fixture("review-fields-misplaced.md")),
        vec![
            (
                Some(21),
                "`Review:` on `AC1` is misplaced; it belongs to a design decision"
            ),
            (
                Some(22),
                "`Review question:` on `AC1` is misplaced; it belongs to a design decision"
            ),
            (
                Some(45),
                "`Review:` on `T01` is misplaced; it belongs to a design decision"
            ),
            (
                Some(46),
                "`Review question:` on `T01` is misplaced; it belongs to a design decision"
            ),
        ]
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

#[cfg(test)]
const PR_REPO: &str = "crocoder-dev/shared-context-engineering";

#[cfg(test)]
const PR_HEAD: &str = "0123456789abcdef0123456789abcdef01234567";

#[cfg(test)]
fn review_plan_source(name: &str, questions: [Option<&str>; 3]) -> String {
    use std::fmt::Write;

    let mut source = format!("# Plan: {name}\n\n## Design decisions\n\n");
    for (index, question) in questions.iter().enumerate() {
        write!(
            source,
            "- D{}: Decision {}.\n  - Boundary: One module.\n  - Rationale: It is simple.\n  - Rejected: Two modules — they split ownership.\n  - Trade-offs: None.\n",
            index + 1,
            index + 1
        )
        .expect("writing to String should not fail");
        if let Some(question) = question {
            write!(
                source,
                "  - Review: required\n  - Review question: {question}\n"
            )
            .expect("writing to String should not fail");
        }
    }
    source.push_str(
        "\n## Acceptance criteria\n\n- [ ] AC1: It works.\n  - Validate: `true`\n  - Scenarios: none (inspection)\n\n## Task stack\n\n- [ ] T01: `Do it` (status:todo)\n  - Task ID: T01\n  - Covers: AC1\n",
    );
    source
}

#[cfg(test)]
fn pr_plan_input(path: &str, source: &str) -> pr_description::PlanInput {
    let report = validate_source(source);
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    pr_description::PlanInput {
        path: path.to_string(),
        plan: extract::extract(source).expect("plan should parse"),
    }
}

#[cfg(test)]
fn pr_arguments(paths: &[&str]) -> pr_description::PrDescriptionArguments {
    pr_description::PrDescriptionArguments {
        repo: PR_REPO.to_string(),
        head: PR_HEAD.to_string(),
        paths: paths.iter().map(ToString::to_string).collect(),
    }
}

#[cfg(test)]
fn write_repository_plan(root: &std::path::Path, path: &str, source: &str) {
    let destination = root.join(path);
    std::fs::create_dir_all(
        destination
            .parent()
            .expect("plan path should have a parent"),
    )
    .expect("plans directory should be created");
    std::fs::write(destination, source).expect("plan should be written");
}

#[cfg(test)]
fn run_pr_description(root: &std::path::Path, paths: &[&str]) -> CommandRun {
    let mut args = vec![NAME, "pr-description", "--repo", PR_REPO, "--head", PR_HEAD];
    args.extend_from_slice(paths);
    run_plan_args(&args, root)
}

#[test]
fn pr_description_golden_multiple_plans() {
    let zeta = pr_plan_input(
        "context/plans/zeta_notes.md",
        &review_plan_source(
            "zeta_notes",
            [None, Some("Is the zeta store shared?"), None],
        ),
    );
    let alpha = pr_plan_input(
        "context/plans/alpha-plan.md",
        &review_plan_source(
            "alpha-plan",
            [Some("Is A acceptable?"), Some("Is B acceptable?"), None],
        ),
    );
    let arguments = pr_arguments(&["context/plans/zeta_notes.md", "context/plans/alpha-plan.md"]);

    let output = pr_description::render(&arguments, &[zeta, alpha]).expect("brief should render");

    let blob = "https://github.com/crocoder-dev/shared-context-engineering/blob/0123456789abcdef0123456789abcdef01234567";
    let expected = format!(
        "<!-- sce:intent-review:start -->\n\
## Intent review\n\
\n\
**Plans:** [alpha-plan]({blob}/context/plans/alpha-plan.md), [zeta\\_notes]({blob}/context/plans/zeta_notes.md)\n\
\n\
**Decisions requiring review**\n\
\n\
1. Is A acceptable? — [alpha-plan D1]({blob}/context/plans/alpha-plan.md#L5)\n\
\n\
2. Is B acceptable? — [alpha-plan D2]({blob}/context/plans/alpha-plan.md#L12)\n\
\n\
3. Is the zeta store shared? — [zeta\\_notes D2]({blob}/context/plans/zeta_notes.md#L10)\n\
\n\
<!-- sce:intent-review:end -->"
    );
    assert_eq!(output, expected);
}

#[test]
fn pr_description_no_flagged_decisions() {
    let plan = pr_plan_input(
        "context/plans/quiet.md",
        &review_plan_source("quiet", [None, None, None]),
    );

    let output = pr_description::render(&pr_arguments(&["context/plans/quiet.md"]), &[plan])
        .expect("brief should render");

    assert_eq!(
        output,
        "<!-- sce:intent-review:start -->\n## Intent review\n\n**Plans:** [quiet](https://github.com/crocoder-dev/shared-context-engineering/blob/0123456789abcdef0123456789abcdef01234567/context/plans/quiet.md)\n\nNo decisions explicitly flagged for review.\n\n<!-- sce:intent-review:end -->"
    );
    assert!(!output.contains("**Decisions requiring review**"));
}

#[test]
fn pr_description_escapes_markdown() {
    let question = r"Keep `code` *bold* _it_ [l] <t> !b |p| ~s & c:\path # (ok) — fine?";
    let expected =
        r"Keep \`code\` \*bold\* \_it\_ \[l\] \<t\> \!b \|p\| \~s \& c:\\path # (ok) — fine?";

    assert_eq!(pr_description::escape_markdown(question), expected);

    let mut unescaped = String::new();
    let mut characters = expected.chars().peekable();
    while let Some(character) = characters.next() {
        let escapes_next = character == '\\'
            && characters
                .peek()
                .is_some_and(|next| r"\`*_[]<>!|~&".contains(*next));
        if !escapes_next {
            unescaped.push(character);
        } else if let Some(next) = characters.next() {
            unescaped.push(next);
        }
    }
    assert_eq!(unescaped, question);

    let plan = pr_plan_input(
        "context/plans/escape.md",
        &review_plan_source("escape", [Some(question), None, None]),
    );
    let output = pr_description::render(&pr_arguments(&["context/plans/escape.md"]), &[plan])
        .expect("brief should render");
    assert!(
        output.contains(&format!("\n1. {expected} — [escape D1](")),
        "{output}"
    );
}

#[test]
fn pr_description_repeated_runs_are_identical() {
    let (_directory, root) = temp_repository();
    write_repository_plan(
        &root,
        "context/plans/beta.md",
        &review_plan_source("beta", [None, None, Some("Is beta safe?")]),
    );
    write_repository_plan(
        &root,
        "context/plans/alpha.md",
        &review_plan_source("alpha", [Some("Is alpha safe?"), None, None]),
    );

    let forward = run_pr_description(&root, &["context/plans/alpha.md", "context/plans/beta.md"]);
    let reverse = run_pr_description(&root, &["context/plans/beta.md", "context/plans/alpha.md"]);

    assert_eq!(
        forward.exit_code,
        std::process::ExitCode::SUCCESS,
        "{}",
        forward.stderr
    );
    assert_eq!(
        reverse.exit_code,
        std::process::ExitCode::SUCCESS,
        "{}",
        reverse.stderr
    );
    assert_eq!(forward.stdout, reverse.stdout);
    assert!(forward
        .stdout
        .starts_with("<!-- sce:intent-review:start -->\n"));
    assert!(
        forward
            .stdout
            .ends_with("#L15)\n\n<!-- sce:intent-review:end -->\n"),
        "{:?}",
        forward.stdout
    );
    assert!(forward.stdout.contains("1. Is alpha safe? — [alpha D1]("));
    assert!(forward.stdout.contains("2. Is beta safe? — [beta D3]("));
}

#[test]
fn pr_description_rejects_invalid_arguments() {
    let (_directory, root) = temp_repository();
    let valid_path = "context/plans/missing.md";
    let mut cases: Vec<(Vec<&str>, &str)> = Vec::new();
    for repo in [
        "owner",
        "owner/",
        "/name",
        "a/b/c",
        "owner/..",
        "owner/.",
        "own_er/name",
        "owner/na me",
    ] {
        cases.push((vec!["--repo", repo, "--head", PR_HEAD, valid_path], repo));
    }
    let short = &PR_HEAD[..39];
    let long = format!("{PR_HEAD}0");
    let upper = PR_HEAD.to_uppercase();
    let non_hex = format!("{}g", &PR_HEAD[..39]);
    for head in [short, long.as_str(), upper.as_str(), non_hex.as_str()] {
        cases.push((vec!["--repo", PR_REPO, "--head", head, valid_path], head));
    }
    let absolute = format!("{}/context/plans/plan.md", root.display());
    for path in [
        absolute.as_str(),
        "docs/plan.md",
        "context/plans/nested/plan.md",
        "context/plans/plan",
        "context/plans/../plan.md",
        "context/plans/..md",
        "context/plans/.md",
        "context/plans/pl@n.md",
    ] {
        cases.push((vec!["--repo", PR_REPO, "--head", PR_HEAD, path], path));
    }
    cases.push((
        vec!["--repo", PR_REPO, "--head", PR_HEAD, valid_path, valid_path],
        valid_path,
    ));

    for (arguments, rejected) in cases {
        let mut args = vec![NAME, "pr-description"];
        args.extend(arguments);
        let run = run_plan_args(&args, &root);

        assert_eq!(
            run.exit_code,
            std::process::ExitCode::from(3),
            "{args:?}: {}",
            run.stderr
        );
        assert!(run.stdout.is_empty(), "{args:?}: {}", run.stdout);
        assert!(
            run.stderr.contains("SCE-ERR-VALIDATION"),
            "{args:?}: {}",
            run.stderr
        );
        assert!(
            run.stderr.contains(&format!("'{rejected}'")),
            "{args:?}: {}",
            run.stderr
        );
    }

    let accepted = run_plan_args(
        &[
            NAME,
            "pr-description",
            "--repo",
            "a-1/x.y_z-1",
            "--head",
            PR_HEAD,
            "context/plans/a.b_c-1.md",
        ],
        &root,
    );
    assert_eq!(
        accepted.exit_code,
        std::process::ExitCode::from(4),
        "{}",
        accepted.stderr
    );
}

#[test]
fn pr_description_rejects_more_than_three_questions() {
    let (_directory, root) = temp_repository();
    write_repository_plan(
        &root,
        "context/plans/beta.md",
        &review_plan_source("beta", [Some("One?"), Some("Two?"), None]),
    );
    write_repository_plan(
        &root,
        "context/plans/alpha.md",
        &review_plan_source("alpha", [Some("Three?"), None, Some("Four?")]),
    );

    let run = run_pr_description(&root, &["context/plans/beta.md", "context/plans/alpha.md"]);

    assert_eq!(
        run.exit_code,
        std::process::ExitCode::from(3),
        "{}",
        run.stderr
    );
    assert!(run.stdout.is_empty(), "{}", run.stdout);
    assert!(run.stderr.contains("Found 4 decisions"), "{}", run.stderr);
    assert!(
        run.stderr.contains("alpha D1, alpha D3, beta D1, beta D2."),
        "{}",
        run.stderr
    );
}

#[test]
fn pr_description_rejects_invalid_plan() {
    let (_directory, root) = temp_repository();
    let path = "context/plans/duplicate-id.md";
    let source = std::fs::read_to_string(fixture_dir().join("duplicate-id.md"))
        .expect("fixture should be readable");
    write_repository_plan(&root, path, &source);

    let run = run_pr_description(&root, &[path]);
    let absolute = root.join(path).display().to_string();
    let check = run_plan_check(&absolute);

    assert_eq!(
        run.exit_code,
        std::process::ExitCode::from(3),
        "{}",
        run.stderr
    );
    assert!(run.stdout.is_empty(), "{}", run.stdout);
    let defects = |stderr: &str, label: &str| -> Vec<String> {
        stderr
            .lines()
            .filter_map(|line| line.strip_prefix(&format!("{label}:")))
            .map(str::to_string)
            .collect()
    };
    assert_eq!(
        defects(&run.stderr, path),
        vec!["29: `S1` is defined more than once".to_string()]
    );
    assert_eq!(
        defects(&run.stderr, path),
        defects(&check.stderr, &absolute)
    );

    let missing = run_pr_description(&root, &["context/plans/absent.md"]);
    assert_eq!(
        missing.exit_code,
        std::process::ExitCode::from(4),
        "{}",
        missing.stderr
    );
    assert!(missing.stdout.is_empty(), "{}", missing.stdout);
    assert!(
        missing.stderr.contains("SCE-ERR-RUNTIME"),
        "{}",
        missing.stderr
    );
}

#[test]
fn plan_pr_description_appears_in_help() {
    let root_help = strip_ansi(&crate::services::help::help_text());
    assert!(
        root_help.contains("sce plan pr-description --repo <owner/name> --head <sha> <path>..."),
        "{root_help}"
    );
    let plan_help = strip_ansi(
        &crate::cli_schema::render_help_for_path(&[NAME]).expect("plan help should be renderable"),
    );
    assert!(plan_help.contains("pr-description"), "{plan_help}");
    let help = strip_ansi(
        &crate::cli_schema::render_help_for_path(&[NAME, "pr-description"])
            .expect("plan pr-description help should be renderable"),
    );
    assert!(help.contains("--repo <OWNER/NAME>"), "{help}");
    assert!(help.contains("--head <SHA>"), "{help}");
    assert!(help.contains("<PATHS>..."), "{help}");
}

#[test]
fn completion_scripts_include_plan_pr_description() {
    use crate::services::completion::{render_completion, CompletionRequest, CompletionShell};

    for shell in [
        CompletionShell::Bash,
        CompletionShell::Zsh,
        CompletionShell::Fish,
    ] {
        let script = render_completion(CompletionRequest { shell });
        assert!(script.contains("pr-description"), "{shell:?}");
    }
}

#[cfg(test)]
fn decision_brief_plan_source(name: &str, decisions: &[(&str, Option<&str>)]) -> String {
    use std::fmt::Write;

    let mut source = format!("# Plan: {name}\n\n## Design decisions\n\n");
    for (index, (statement, question)) in decisions.iter().enumerate() {
        if statement.is_empty() {
            writeln!(source, "- D{}:", index + 1)
        } else {
            writeln!(source, "- D{}: {statement}", index + 1)
        }
        .expect("writing to String should not fail");
        source.push_str(
            "  - Boundary: One module.\n  - Rationale: It is simple.\n  - Rejected: Two modules — they split ownership.\n  - Trade-offs: None.\n",
        );
        if let Some(question) = question {
            write!(
                source,
                "  - Review: required\n  - Review question: {question}\n"
            )
            .expect("writing to String should not fail");
        }
    }
    source.push_str(
        "\n## Acceptance criteria\n\n- [ ] AC1: It works.\n  - Validate: `true`\n  - Scenarios: none (inspection)\n\n## Task stack\n\n- [ ] T01: `Do it` (status:todo)\n  - Task ID: T01\n  - Covers: AC1\n",
    );
    source
}

#[cfg(test)]
fn render_decision_brief(
    paths: &[&str],
    plans: &[pr_description::PlanInput],
) -> Result<String, crate::services::error::CliError> {
    pr_description::render_format(
        &pr_arguments(paths),
        plans,
        pr_description::BriefFormat::DecisionBrief,
    )
}

#[test]
fn decision_brief_golden_two_decisions() {
    let plan = pr_plan_input(
        "context/plans/brief.md",
        &decision_brief_plan_source(
            "brief",
            &[
                ("Keep one store.", Some("Is one store enough?")),
                ("Log every read.", None),
                (
                    "Read plans from the\n  head commit   only.",
                    Some("Should reads ignore the working tree?"),
                ),
            ],
        ),
    );

    let output = render_decision_brief(&["context/plans/brief.md"], &[plan])
        .expect("decision brief should render");

    let blob = "https://github.com/crocoder-dev/shared-context-engineering/blob/0123456789abcdef0123456789abcdef01234567";
    let expected = format!(
        "<!-- sce:intent-review:start -->\n\
## Decision Brief\n\
\n\
**Review requested:** 2 decisions\n\
\n\
**Plans:** [brief]({blob}/context/plans/brief.md)\n\
\n\
### 1. Is one store enough?\n\
\n\
**Proposed:** Keep one store.\n\
\n\
[Review brief D1 — rationale, alternatives and trade-offs]({blob}/context/plans/brief.md#L5)\n\
\n\
### 2. Should reads ignore the working tree?\n\
\n\
**Proposed:** Read plans from the head commit only.\n\
\n\
[Review brief D3 — rationale, alternatives and trade-offs]({blob}/context/plans/brief.md#L17)\n\
\n\
<!-- sce:intent-review:end -->\n"
    );
    assert_eq!(output, expected);
    assert!(!output.contains("\n\n\n"), "{output:?}");
    assert!(!output.ends_with("\n\n"), "{output:?}");
}

#[test]
fn decision_brief_golden_one_decision() {
    let plan = pr_plan_input(
        "context/plans/example.md",
        &decision_brief_plan_source(
            "example",
            &[(
                "Use one shared store for all sessions.",
                Some("Is this architectural choice acceptable?"),
            )],
        ),
    );

    let output = render_decision_brief(&["context/plans/example.md"], &[plan])
        .expect("decision brief should render");

    let plan_url = "https://github.com/crocoder-dev/shared-context-engineering/blob/0123456789abcdef0123456789abcdef01234567/context/plans/example.md";
    let expected = format!(
        "<!-- sce:intent-review:start -->\n\
## Decision Brief\n\
\n\
**Review requested:** 1 decision\n\
\n\
**Plans:** [example]({plan_url})\n\
\n\
### 1. Is this architectural choice acceptable?\n\
\n\
**Proposed:** Use one shared store for all sessions.\n\
\n\
[Review example D1 — rationale, alternatives and trade-offs]({plan_url}#L5)\n\
\n\
<!-- sce:intent-review:end -->\n"
    );
    assert_eq!(output, expected);
    assert!(!output.contains("\n\n\n"), "{output:?}");
    assert!(!output.ends_with("\n\n"), "{output:?}");
}

#[test]
fn decision_brief_stdout_ends_with_one_newline() {
    let plan = pr_plan_input(
        "context/plans/example.md",
        &decision_brief_plan_source(
            "example",
            &[(
                "Use one shared store for all sessions.",
                Some("Is this architectural choice acceptable?"),
            )],
        ),
    );
    let brief = render_decision_brief(&["context/plans/example.md"], &[plan])
        .expect("decision brief should render");

    let run = render_plan_result(
        Ok(pr_description::decision_brief_stdout_payload(brief.clone())),
        Vec::new(),
    );

    assert_eq!(run.exit_code, std::process::ExitCode::SUCCESS);
    assert_eq!(run.stdout, brief);
    assert!(run.stdout.ends_with('\n'), "{:?}", run.stdout);
    assert!(!run.stdout.ends_with("\n\n"), "{:?}", run.stdout);
}

#[test]
fn decision_brief_no_marked_decisions() {
    let plan = pr_plan_input(
        "context/plans/quiet.md",
        &decision_brief_plan_source("quiet", &[("Keep it.", None), ("Log it.", None)]),
    );

    let output = render_decision_brief(&["context/plans/quiet.md"], &[plan])
        .expect("decision brief should render");

    assert_eq!(
        output,
        "<!-- sce:intent-review:start -->\n## Decision Brief\n\n**Review requested:** 0 decisions\n\n**Plans:** [quiet](https://github.com/crocoder-dev/shared-context-engineering/blob/0123456789abcdef0123456789abcdef01234567/context/plans/quiet.md)\n\nNo decisions explicitly flagged for review.\n\n<!-- sce:intent-review:end -->\n"
    );
    assert!(!output.contains("###"), "{output}");
    let lowered = output.to_lowercase();
    for claim in ["approv", "safe", "verified"] {
        assert!(!lowered.contains(claim), "{output}");
    }
}

#[test]
fn decision_brief_escapes_markdown() {
    let statement = r"Use `x` *y* _z_ [a] <b> |c| d:\e now";
    let question = r"Is `q` *ok* _here_ [x] <y> |z| a:\b?";
    let escaped_statement = r"Use \`x\` \*y\* \_z\_ \[a\] \<b\> \|c\| d:\\e now";
    let escaped_question = r"Is \`q\` \*ok\* \_here\_ \[x\] \<y\> \|z\| a:\\b?";

    let unescape = |text: &str| {
        let mut unescaped = String::new();
        let mut characters = text.chars().peekable();
        while let Some(character) = characters.next() {
            let escapes_next = character == '\\'
                && characters
                    .peek()
                    .is_some_and(|next| r"\`*_[]<>|".contains(*next));
            if !escapes_next {
                unescaped.push(character);
            } else if let Some(next) = characters.next() {
                unescaped.push(next);
            }
        }
        unescaped
    };
    assert_eq!(unescape(escaped_statement), statement);
    assert_eq!(unescape(escaped_question), question);

    let plan = pr_plan_input(
        "context/plans/escape.md",
        &decision_brief_plan_source("escape", &[(statement, Some(question))]),
    );
    let output = render_decision_brief(&["context/plans/escape.md"], &[plan])
        .expect("decision brief should render");

    assert!(
        output.contains("\n\n**Review requested:** 1 decision\n\n"),
        "{output}"
    );
    assert!(
        output.contains(&format!("\n\n### 1. {escaped_question}\n\n")),
        "{output}"
    );
    assert!(
        output.contains(&format!("\n\n**Proposed:** {escaped_statement}\n\n")),
        "{output}"
    );
}

#[test]
fn decision_brief_orders_plans_deterministically() {
    let zeta = || {
        pr_plan_input(
            "context/plans/zeta.md",
            &decision_brief_plan_source("zeta", &[("Z one.", Some("Is Z one right?"))]),
        )
    };
    let alpha = || {
        pr_plan_input(
            "context/plans/alpha.md",
            &decision_brief_plan_source(
                "alpha",
                &[
                    ("A one.", None),
                    ("A two.", Some("Is A two right?")),
                    ("A three.", Some("Is A three right?")),
                ],
            ),
        )
    };
    let reverse_paths = ["context/plans/zeta.md", "context/plans/alpha.md"];
    let sorted_paths = ["context/plans/alpha.md", "context/plans/zeta.md"];

    let first = render_decision_brief(&reverse_paths, &[zeta(), alpha()])
        .expect("decision brief should render");
    let second = render_decision_brief(&reverse_paths, &[zeta(), alpha()])
        .expect("decision brief should render");
    let sorted = render_decision_brief(&sorted_paths, &[alpha(), zeta()])
        .expect("decision brief should render");

    assert_eq!(first, second);
    assert_eq!(first, sorted);
    assert!(
        first.contains("\n\n**Review requested:** 3 decisions\n\n"),
        "{first}"
    );

    let blob = "https://github.com/crocoder-dev/shared-context-engineering/blob/0123456789abcdef0123456789abcdef01234567";
    assert!(
        first.contains(&format!(
            "**Plans:** [alpha]({blob}/context/plans/alpha.md), [zeta]({blob}/context/plans/zeta.md)\n"
        )),
        "{first}"
    );
    let ordered = [
        "### 1. Is A two right?",
        "[Review alpha D2 — rationale, alternatives and trade-offs]",
        "alpha.md#L10)",
        "### 2. Is A three right?",
        "[Review alpha D3 — rationale, alternatives and trade-offs]",
        "alpha.md#L17)",
        "### 3. Is Z one right?",
        "[Review zeta D1 — rationale, alternatives and trade-offs]",
        "zeta.md#L5)",
    ];
    let mut cursor = 0;
    for needle in ordered {
        let found = first[cursor..]
            .find(needle)
            .unwrap_or_else(|| panic!("missing {needle:?} after byte {cursor}: {first}"));
        cursor += found + needle.len();
    }
}

#[test]
fn decision_brief_rejects_empty_statement() {
    let source = decision_brief_plan_source(
        "empty",
        &[
            ("Keep one store.", Some("Is one store enough?")),
            ("", Some("Is the blank decision right?")),
        ],
    );
    assert!(source.contains("\n- D2:\n"), "{source}");

    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let plan_path = directory.path().join("empty.md");
    std::fs::write(&plan_path, &source).expect("plan should be written");
    let check = run_plan_check(plan_path.to_str().expect("path should be UTF-8"));
    assert_eq!(
        check.exit_code,
        std::process::ExitCode::SUCCESS,
        "{}",
        check.stderr
    );

    let plan = pr_plan_input("context/plans/empty.md", &source);
    let paths = ["context/plans/empty.md"];

    let error = render_decision_brief(&paths, std::slice::from_ref(&plan))
        .expect_err("empty statement should fail the decision brief");
    assert_eq!(error.class().exit_code(), 3);
    let message = error.to_string();
    assert!(message.contains("'empty'"), "{message}");
    assert!(message.contains("D2"), "{message}");
    assert!(!message.contains("D1"), "{message}");

    let intent_review = pr_description::render(&pr_arguments(&paths), &[plan])
        .expect("intent review should still render");
    assert!(
        intent_review.contains("2. Is the blank decision right? — [empty D2]("),
        "{intent_review}"
    );
}
