pub mod claims;
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
        stderr: strip_ansi(&String::from_utf8(stderr).expect("stderr should be UTF-8")),
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
**Review scope:** 3 architectural decisions require senior review. Consider moving independent changes into a separate PR to keep this review focused.\n\
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
fn pr_description_golden_two_decisions() {
    let plan = pr_plan_input(
        "context/plans/alpha-plan.md",
        &review_plan_source(
            "alpha-plan",
            [Some("Is A acceptable?"), Some("Is B acceptable?"), None],
        ),
    );

    let output = pr_description::render(&pr_arguments(&["context/plans/alpha-plan.md"]), &[plan])
        .expect("brief should render");

    let plan_url = "https://github.com/crocoder-dev/shared-context-engineering/blob/0123456789abcdef0123456789abcdef01234567/context/plans/alpha-plan.md";
    let expected = format!(
        "<!-- sce:intent-review:start -->\n\
## Intent review\n\
\n\
**Plans:** [alpha-plan]({plan_url})\n\
\n\
**Decisions requiring review**\n\
\n\
1. Is A acceptable? — [alpha-plan D1]({plan_url}#L5)\n\
\n\
2. Is B acceptable? — [alpha-plan D2]({plan_url}#L12)\n\
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
fn pr_description_accepts_four_questions_across_plans() {
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
    for path in ["context/plans/beta.md", "context/plans/alpha.md"] {
        let check = run_plan_check(root.join(path).to_str().expect("path should be UTF-8"));
        assert_eq!(
            check.exit_code,
            std::process::ExitCode::SUCCESS,
            "{}",
            check.stderr
        );
    }

    let run = run_pr_description(&root, &["context/plans/beta.md", "context/plans/alpha.md"]);

    assert_eq!(
        run.exit_code,
        std::process::ExitCode::SUCCESS,
        "{}",
        run.stderr
    );
    assert!(run.stderr.is_empty(), "{}", run.stderr);
    assert_eq!(
        run.stdout.matches("**Review scope:**").count(),
        1,
        "{}",
        run.stdout
    );
    assert!(
        run.stdout
            .contains("**Review scope:** 4 architectural decisions require senior review."),
        "{}",
        run.stdout
    );
    let numbered: Vec<&str> = run
        .stdout
        .lines()
        .filter(|line| line.starts_with(|first: char| first.is_ascii_digit()))
        .map(|line| line.split(" — ").next().unwrap_or(line))
        .collect();
    assert_eq!(
        numbered,
        vec!["1. Three?", "2. Four?", "3. One?", "4. Two?"],
        "{}",
        run.stdout
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
    assert_eq!(first.matches("**Review scope:**").count(), 1, "{first}");
    assert!(
        first.contains("\n\n**Review scope:** 3 architectural decisions require senior review. Consider moving independent changes into a separate PR to keep this review focused.\n\n### 1. "),
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

#[cfg(test)]
fn scope_warning(count: usize) -> String {
    format!(
        "**Review scope:** {count} architectural decisions require senior review. Consider moving independent changes into a separate PR to keep this review focused."
    )
}

#[cfg(test)]
fn scope_warning_plans() -> Vec<pr_description::PlanInput> {
    vec![
        pr_plan_input(
            "context/plans/gamma.md",
            &decision_brief_plan_source("gamma", &[("G one.", Some("Is G one right?"))]),
        ),
        pr_plan_input(
            "context/plans/alpha.md",
            &decision_brief_plan_source(
                "alpha",
                &[
                    ("A one.", Some("Is A one right?")),
                    ("A two.", None),
                    ("A three.", Some("Is A three right?")),
                ],
            ),
        ),
        pr_plan_input(
            "context/plans/beta.md",
            &decision_brief_plan_source(
                "beta",
                &[("B one.", None), ("B two.", Some("Is B two right?"))],
            ),
        ),
    ]
}

#[test]
fn decision_brief_scope_warning_spans_all_plans() {
    let paths = [
        "context/plans/gamma.md",
        "context/plans/alpha.md",
        "context/plans/beta.md",
    ];
    let mut reversed_paths = paths;
    reversed_paths.reverse();
    let mut reversed_plans = scope_warning_plans();
    reversed_plans.reverse();

    let blob = "https://github.com/crocoder-dev/shared-context-engineering/blob/0123456789abcdef0123456789abcdef01234567";
    let plans_line = format!(
        "**Plans:** [alpha]({blob}/context/plans/alpha.md), [beta]({blob}/context/plans/beta.md), [gamma]({blob}/context/plans/gamma.md)"
    );
    let warning = scope_warning(4);

    let brief = render_decision_brief(&paths, &scope_warning_plans())
        .expect("decision brief should render");
    let expected_brief = format!(
        "<!-- sce:intent-review:start -->\n\
## Decision Brief\n\
\n\
**Review requested:** 4 decisions\n\
\n\
{plans_line}\n\
\n\
{warning}\n\
\n\
### 1. Is A one right?\n\
\n\
**Proposed:** A one.\n\
\n\
[Review alpha D1 — rationale, alternatives and trade-offs]({blob}/context/plans/alpha.md#L5)\n\
\n\
### 2. Is A three right?\n\
\n\
**Proposed:** A three.\n\
\n\
[Review alpha D3 — rationale, alternatives and trade-offs]({blob}/context/plans/alpha.md#L17)\n\
\n\
### 3. Is B two right?\n\
\n\
**Proposed:** B two.\n\
\n\
[Review beta D2 — rationale, alternatives and trade-offs]({blob}/context/plans/beta.md#L10)\n\
\n\
### 4. Is G one right?\n\
\n\
**Proposed:** G one.\n\
\n\
[Review gamma D1 — rationale, alternatives and trade-offs]({blob}/context/plans/gamma.md#L5)\n\
\n\
<!-- sce:intent-review:end -->\n"
    );
    assert_eq!(brief, expected_brief);
    assert_eq!(
        render_decision_brief(&reversed_paths, &reversed_plans)
            .expect("decision brief should render"),
        brief
    );

    let intent_review = pr_description::render(&pr_arguments(&paths), &scope_warning_plans())
        .expect("intent review should render");
    let expected_intent_review = format!(
        "<!-- sce:intent-review:start -->\n\
## Intent review\n\
\n\
{plans_line}\n\
\n\
{warning}\n\
\n\
**Decisions requiring review**\n\
\n\
1. Is A one right? — [alpha D1]({blob}/context/plans/alpha.md#L5)\n\
\n\
2. Is A three right? — [alpha D3]({blob}/context/plans/alpha.md#L17)\n\
\n\
3. Is B two right? — [beta D2]({blob}/context/plans/beta.md#L10)\n\
\n\
4. Is G one right? — [gamma D1]({blob}/context/plans/gamma.md#L5)\n\
\n\
<!-- sce:intent-review:end -->"
    );
    assert_eq!(intent_review, expected_intent_review);
    assert_eq!(
        pr_description::render(&pr_arguments(&reversed_paths), &reversed_plans)
            .expect("intent review should render"),
        intent_review
    );
}

#[cfg(test)]
fn scope_threshold_plans(
    names: &[&str],
    count: usize,
) -> (
    Vec<pr_description::PlanInput>,
    Vec<(String, String, String)>,
) {
    let blob = "https://github.com/crocoder-dev/shared-context-engineering/blob/0123456789abcdef0123456789abcdef01234567";
    let mut plans = Vec::new();
    let mut marked = Vec::new();
    for (plan_index, name) in names.iter().enumerate() {
        let questions: Vec<String> = (1..=2)
            .map(|decision| format!("Is {name} D{decision} right?"))
            .collect();
        let decisions: Vec<(&str, Option<&str>)> = questions
            .iter()
            .enumerate()
            .map(|(index, question)| {
                (
                    "Keep it.",
                    (plan_index * 2 + index < count).then_some(question.as_str()),
                )
            })
            .collect();
        let source = decision_brief_plan_source(name, &decisions);
        for (index, (_, question)) in decisions.iter().enumerate() {
            if let Some(question) = question {
                let id = format!("D{}", index + 1);
                let line = source_line(&source, &format!("- {id}:"));
                marked.push((
                    question.to_string(),
                    format!("{name} {id}"),
                    format!("{blob}/context/plans/{name}.md#L{line}"),
                ));
            }
        }
        plans.push(pr_plan_input(&format!("context/plans/{name}.md"), &source));
    }
    (plans, marked)
}

#[test]
fn decision_brief_scope_warning_threshold_keeps_every_decision() {
    use pr_description::BriefFormat;

    let blob = "https://github.com/crocoder-dev/shared-context-engineering/blob/0123456789abcdef0123456789abcdef01234567";
    let names = ["alpha", "bravo", "charlie", "delta"];
    let paths: Vec<String> = names
        .iter()
        .map(|name| format!("context/plans/{name}.md"))
        .collect();
    let paths: Vec<&str> = paths.iter().map(String::as_str).collect();
    let mut reversed_paths = paths.clone();
    reversed_paths.reverse();
    let plan_links: Vec<String> = names
        .iter()
        .map(|name| format!("[{name}]({blob}/context/plans/{name}.md)"))
        .collect();
    let plans_line = format!("**Plans:** {}", plan_links.join(", "));

    for count in 0..=names.len() * 2 {
        let (plans, marked) = scope_threshold_plans(&names, count);
        assert_eq!(marked.len(), count);
        let mut reversed_plans = plans.clone();
        reversed_plans.reverse();

        for format in [BriefFormat::IntentReview, BriefFormat::DecisionBrief] {
            let case = format!("{format:?} with {count} marked decisions");
            let output = pr_description::render_format(&pr_arguments(&paths), &plans, format)
                .unwrap_or_else(|error| panic!("{case}: {error}"));
            let reversed = pr_description::render_format(
                &pr_arguments(&reversed_paths),
                &reversed_plans,
                format,
            )
            .unwrap_or_else(|error| panic!("{case}: {error}"));
            assert_eq!(output, reversed, "{case}");

            let mut needles = vec![plans_line.clone()];
            if count >= 3 {
                assert_eq!(
                    output.matches("**Review scope:**").count(),
                    1,
                    "{case}: {output}"
                );
                needles.push(format!("\n\n{}\n", scope_warning(count)));
            } else {
                assert!(!output.contains("**Review scope:**"), "{case}: {output}");
            }
            for (number, (question, label, url)) in marked.iter().enumerate() {
                let number = number + 1;
                assert_eq!(
                    output.matches(question.as_str()).count(),
                    1,
                    "{case}: {output}"
                );
                match format {
                    BriefFormat::IntentReview => {
                        needles.push(format!("\n{number}. {question} — [{label}]({url})\n"));
                    }
                    BriefFormat::DecisionBrief => {
                        needles.push(format!("\n### {number}. {question}\n"));
                        needles.push(format!(
                            "\n[Review {label} — rationale, alternatives and trade-offs]({url})\n"
                        ));
                    }
                }
            }
            let mut cursor = 0;
            for needle in &needles {
                let found = output[cursor..].find(needle.as_str()).unwrap_or_else(|| {
                    panic!("{case}: missing {needle:?} after byte {cursor}: {output}")
                });
                cursor += found + needle.len();
            }

            let numbered = output
                .lines()
                .filter(|line| match format {
                    BriefFormat::IntentReview => {
                        line.starts_with(|first: char| first.is_ascii_digit())
                    }
                    BriefFormat::DecisionBrief => line.starts_with("### "),
                })
                .count();
            assert_eq!(numbered, count, "{case}: {output}");
        }
    }
}

#[cfg(test)]
fn git_output(root: &std::path::Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .args([
            "-c",
            "user.name=SCE Test",
            "-c",
            "user.email=sce-test@example.com",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
        ])
        .args(args)
        .current_dir(root)
        .output()
        .expect("git should run");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("git output should be UTF-8")
        .trim_end()
        .to_string()
}

#[cfg(test)]
fn commit_repository(root: &std::path::Path) -> String {
    git_output(root, &["add", "-A"]);
    git_output(root, &["commit", "--quiet", "--no-verify", "-m", "plans"]);
    git_output(root, &["rev-parse", "HEAD"])
}

#[cfg(test)]
fn run_review(root: &std::path::Path, head: &str, paths: &[&str]) -> CommandRun {
    let mut args = vec![NAME, "review", "--repo", PR_REPO, "--head", head];
    args.extend_from_slice(paths);
    run_plan_args(&args, root)
}

#[cfg(test)]
fn assert_failure(run: &CommandRun, exit_code: u8, case: &str) {
    assert_eq!(
        run.exit_code,
        std::process::ExitCode::from(exit_code),
        "{case}: {}",
        run.stderr
    );
    assert!(run.stdout.is_empty(), "{case}: {}", run.stdout);
}

#[test]
fn review_reads_committed_plan() {
    let (_directory, root) = temp_repository();
    let path = "context/plans/committed.md";
    let source = decision_brief_plan_source(
        "committed",
        &[("Keep one store.", Some("Is one store enough?"))],
    );
    write_repository_plan(&root, path, &source);
    let head = commit_repository(&root);
    let url = format!("https://github.com/{PR_REPO}/blob/{head}/{path}");
    let expected = format!(
        "<!-- sce:intent-review:start -->\n## Decision Brief\n\n**Review requested:** 1 decision\n\n**Plans:** [committed]({url})\n\n### 1. Is one store enough?\n\n**Proposed:** Keep one store.\n\n[Review committed D1 — rationale, alternatives and trade-offs]({url}#L5)\n\n<!-- sce:intent-review:end -->\n"
    );

    let before = run_review(&root, &head, &[path]);
    assert_eq!(
        before.exit_code,
        std::process::ExitCode::SUCCESS,
        "{}",
        before.stderr
    );
    assert_eq!(before.stdout, expected);
    assert!(before.stdout.ends_with('\n'), "{:?}", before.stdout);
    assert!(!before.stdout.ends_with("\n\n"), "{:?}", before.stdout);

    let edited = source
        .replacen(
            "## Design decisions\n",
            "Inserted.\n\nMore.\n\n## Design decisions\n",
            1,
        )
        .replace("Is one store enough?", "Is the edited question shown?");
    write_repository_plan(&root, path, &edited);

    let after = run_review(&root, &head, &[path]);
    assert_eq!(
        after.exit_code,
        std::process::ExitCode::SUCCESS,
        "{}",
        after.stderr
    );
    assert_eq!(after.stdout, before.stdout);
    assert!(
        !after.stdout.contains("edited question"),
        "{}",
        after.stdout
    );
}

#[test]
fn review_renders_every_marked_decision() {
    let (_directory, root) = temp_repository();
    write_repository_plan(
        &root,
        "context/plans/beta.md",
        &review_plan_source("beta", [Some("One?"), Some("Two?"), None]),
    );
    write_repository_plan(
        &root,
        "context/plans/alpha.md",
        &review_plan_source("alpha", [Some("Three?"), None, None]),
    );
    let three = commit_repository(&root);

    let accepted = run_review(
        &root,
        &three,
        &["context/plans/beta.md", "context/plans/alpha.md"],
    );
    assert_eq!(
        accepted.exit_code,
        std::process::ExitCode::SUCCESS,
        "{}",
        accepted.stderr
    );
    assert!(
        accepted
            .stdout
            .contains("**Review requested:** 3 decisions"),
        "{}",
        accepted.stdout
    );
    let headings: Vec<&str> = accepted
        .stdout
        .lines()
        .filter(|line| line.starts_with("### "))
        .collect();
    assert_eq!(
        headings,
        vec!["### 1. Three?", "### 2. One?", "### 3. Two?"]
    );

    write_repository_plan(
        &root,
        "context/plans/alpha.md",
        &review_plan_source("alpha", [Some("Three?"), None, Some("Four?")]),
    );
    let four = commit_repository(&root);

    let accepted = run_review(
        &root,
        &four,
        &["context/plans/beta.md", "context/plans/alpha.md"],
    );
    assert_eq!(
        accepted.exit_code,
        std::process::ExitCode::SUCCESS,
        "{}",
        accepted.stderr
    );
    assert!(accepted.stderr.is_empty(), "{}", accepted.stderr);
    assert_eq!(
        accepted.stdout.matches("**Review scope:**").count(),
        1,
        "{}",
        accepted.stdout
    );
    assert!(
        accepted.stdout.contains(&scope_warning(4)),
        "{}",
        accepted.stdout
    );
    let headings: Vec<&str> = accepted
        .stdout
        .lines()
        .filter(|line| line.starts_with("### "))
        .collect();
    assert_eq!(
        headings,
        vec![
            "### 1. Three?",
            "### 2. Four?",
            "### 3. One?",
            "### 4. Two?"
        ]
    );
}

#[test]
fn review_claims_ignore_review_scope() {
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
    let head = commit_repository(&root);

    let claims = run_review_claims(
        &root,
        &head,
        &["context/plans/beta.md", "context/plans/alpha.md"],
    );
    assert_eq!(
        claims.exit_code,
        std::process::ExitCode::SUCCESS,
        "{}",
        claims.stderr
    );
    assert!(!claims.stdout.contains("Review scope"), "{}", claims.stdout);
    let document: serde_json::Value =
        serde_json::from_str(&claims.stdout).expect("claims should be JSON");
    assert_eq!(
        document["claims"].as_array().map(Vec::len),
        Some(2),
        "{}",
        claims.stdout
    );
}

#[test]
fn review_rejects_invalid_git_inputs() {
    use std::io::Write;

    let (_directory, root) = temp_repository();
    let valid = "context/plans/valid.md";
    write_repository_plan(
        &root,
        valid,
        &decision_brief_plan_source("valid", &[("Keep one store.", Some("Is it enough?"))]),
    );
    let invalid = "context/plans/invalid.md";
    write_repository_plan(
        &root,
        invalid,
        &std::fs::read_to_string(fixture_dir().join("duplicate-id.md"))
            .expect("fixture should be readable"),
    );
    let linked = "context/plans/linked.md";
    std::os::unix::fs::symlink("valid.md", root.join(linked)).expect("symlink should be created");
    let binary = "context/plans/binary.md";
    let mut bytes = b"# Plan: binary\n\n".to_vec();
    bytes.extend_from_slice(&[0xff, 0xfe, b'\n']);
    std::fs::File::create(root.join(binary))
        .and_then(|mut file| file.write_all(&bytes))
        .expect("binary plan should be written");
    let head = commit_repository(&root);
    let unknown = "0123456789abcdef0123456789abcdef01234567";
    assert_ne!(head, unknown);

    let run_with = |repo: &str, head: &str, paths: &[&str]| {
        let mut args = vec![NAME, "review", "--repo", repo, "--head", head];
        args.extend_from_slice(paths);
        run_plan_args(&args, &root)
    };

    let cases: [(&str, CommandRun, u8); 10] = [
        ("invalid repo", run_with("not-a-repo", &head, &[valid]), 3),
        ("short head", run_with(PR_REPO, &head[..12], &[valid]), 3),
        (
            "uppercase head",
            run_with(PR_REPO, &head.to_uppercase(), &[valid]),
            3,
        ),
        ("unknown commit", run_with(PR_REPO, unknown, &[valid]), 4),
        (
            "absent path",
            run_with(PR_REPO, &head, &["context/plans/absent.md"]),
            4,
        ),
        ("outside plans", run_with(PR_REPO, &head, &["README.md"]), 3),
        (
            "duplicate path",
            run_with(PR_REPO, &head, &[valid, valid]),
            3,
        ),
        ("invalid plan", run_with(PR_REPO, &head, &[invalid]), 3),
        ("symlink", run_with(PR_REPO, &head, &[linked]), 4),
        ("non-UTF-8", run_with(PR_REPO, &head, &[binary]), 4),
    ];
    for (case, run, exit_code) in &cases {
        assert_failure(run, *exit_code, case);
    }

    let [_, _, _, (_, unknown_run, _), (_, absent_run, _), _, _, (_, invalid_run, _), (_, symlink_run, _), (_, binary_run, _)] =
        &cases;
    assert!(
        unknown_run
            .stderr
            .contains("is not available in the local repository"),
        "{}",
        unknown_run.stderr
    );
    assert!(
        absent_run.stderr.contains("does not exist at commit"),
        "{}",
        absent_run.stderr
    );
    assert!(
        invalid_run
            .stderr
            .contains(&format!("{invalid}:29: `S1` is defined more than once")),
        "{}",
        invalid_run.stderr
    );
    assert!(
        symlink_run
            .stderr
            .contains("is not a regular file (mode 120000"),
        "{}",
        symlink_run.stderr
    );
    assert!(
        binary_run.stderr.contains("is not valid UTF-8"),
        "{}",
        binary_run.stderr
    );
}

#[cfg(test)]
fn run_cli_args(args: &[&str], start: &std::path::Path) -> CommandRun {
    let mut stderr = Vec::new();
    let result = crate::services::parse::command_runtime::parse_runtime_command(
        std::iter::once("sce")
            .chain(args.iter().copied())
            .map(String::from),
        &crate::services::command_registry::CommandRegistry::default(),
        None,
    )
    .and_then(|command| match command {
        crate::services::command_registry::RuntimeCommand::HelpText(command) => Ok(command.text),
        crate::services::command_registry::RuntimeCommand::Plan(command) => {
            command.execute_with(|| Ok(start.to_path_buf()), &mut stderr)
        }
        _ => panic!("expected help text or plan command"),
    });
    render_plan_result(result, stderr)
}

#[test]
fn review_removed_name_shows_plan_help() {
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
    let head = commit_repository(&root);
    let paths = ["context/plans/beta.md", "context/plans/alpha.md"];

    let plan_help = run_cli_args(&[NAME, "--help"], &root);
    assert_eq!(plan_help.exit_code, std::process::ExitCode::SUCCESS);
    assert!(plan_help.stderr.is_empty(), "{}", plan_help.stderr);
    assert!(plan_help.stdout.contains("review"), "{}", plan_help.stdout);

    let mut removed_args = vec![NAME, "decision-brief", "--repo", PR_REPO, "--head", &head];
    removed_args.extend_from_slice(&paths);
    let removed = run_cli_args(&removed_args, &root);
    assert_eq!(removed.exit_code, std::process::ExitCode::SUCCESS);
    assert_eq!(removed.stdout, plan_help.stdout);
    assert!(removed.stderr.is_empty(), "{}", removed.stderr);
    assert!(
        !removed.stdout.contains("## Decision Brief"),
        "{}",
        removed.stdout
    );

    let mut review_args = vec![NAME, "review", "--repo", PR_REPO, "--head", &head];
    review_args.extend_from_slice(&paths);
    let review = run_cli_args(&review_args, &root);
    assert_eq!(
        review.exit_code,
        std::process::ExitCode::SUCCESS,
        "{}",
        review.stderr
    );
    assert!(
        review.stdout.contains(&scope_warning(4)),
        "{}",
        review.stdout
    );
}

#[test]
fn review_appears_in_help_and_completions() {
    use crate::services::completion::{render_completion, CompletionRequest, CompletionShell};

    let root_help = strip_ansi(&crate::services::help::help_text());
    assert!(
        root_help.contains("sce plan review [--claims] --repo <owner/name> --head <sha> <path>..."),
        "{root_help}"
    );
    assert!(!root_help.contains("decision-brief"), "{root_help}");

    let plan_help = strip_ansi(
        &crate::cli_schema::render_help_for_path(&[NAME]).expect("plan help should be renderable"),
    );
    assert!(
        plan_help
            .lines()
            .any(|line| line.trim_start().starts_with("review ")),
        "{plan_help}"
    );
    assert!(!plan_help.contains("decision-brief"), "{plan_help}");

    let review_help = strip_ansi(
        &crate::cli_schema::render_help_for_path(&[NAME, "review"])
            .expect("review help should be renderable"),
    );
    assert!(review_help.contains("--repo <OWNER/NAME>"), "{review_help}");
    assert!(review_help.contains("--head <SHA>"), "{review_help}");
    assert!(review_help.contains("<PATHS>..."), "{review_help}");
    assert!(review_help.contains("--claims"), "{review_help}");
    assert!(!review_help.contains("decision-brief"), "{review_help}");

    for (shell, needle, claims_needle) in [
        (
            CompletionShell::Bash,
            "check init pr-description review",
            "opts=\"-h --claims --repo --head --help\"",
        ),
        (
            CompletionShell::Zsh,
            "'review:Print a Markdown Decision Brief",
            "'--claims[Print the sce.plan.claims.v1 JSON claims manifest",
        ),
        (
            CompletionShell::Fish,
            "-f -a \"review\"",
            "__fish_seen_subcommand_from review\" -l claims",
        ),
    ] {
        let script = render_completion(CompletionRequest { shell });
        assert!(script.contains(needle), "{shell:?}");
        assert!(script.contains(claims_needle), "{shell:?}");
        assert!(!script.contains("decision-brief"), "{shell:?}");
        assert!(script.contains("pr-description"), "{shell:?}");
    }
}

#[cfg(test)]
fn run_review_claims(root: &std::path::Path, head: &str, paths: &[&str]) -> CommandRun {
    let mut args = vec![
        NAME, "review", "--claims", "--repo", PR_REPO, "--head", head,
    ];
    args.extend_from_slice(paths);
    run_plan_args(&args, root)
}

#[cfg(test)]
fn source_line(source: &str, prefix: &str) -> u64 {
    let index = source
        .lines()
        .position(|line| line.starts_with(prefix))
        .expect("prefix should appear in the source");
    u64::try_from(index + 1).expect("line should fit in u64")
}

#[test]
fn review_claims_reads_committed_plans() {
    let (_directory, root) = temp_repository();
    let beta = "context/plans/beta.md";
    let alpha = "context/plans/alpha.md";
    let beta_source = decision_brief_plan_source("beta", &[("Keep one store.", None)]);
    let alpha_source = decision_brief_plan_source(
        "alpha",
        &[
            ("Use one queue.", Some("Is one queue enough?")),
            ("Retry twice.", None),
        ],
    );
    write_repository_plan(&root, beta, &beta_source);
    write_repository_plan(&root, alpha, &alpha_source);
    let head = commit_repository(&root);

    let forward = run_review_claims(&root, &head, &[beta, alpha]);
    assert_eq!(
        forward.exit_code,
        std::process::ExitCode::SUCCESS,
        "{}",
        forward.stderr
    );
    let reverse = run_review_claims(&root, &head, &[alpha, beta]);
    assert_eq!(reverse.stdout, forward.stdout);

    write_repository_plan(
        &root,
        alpha,
        &alpha_source
            .replacen(
                "## Acceptance criteria\n",
                "Inserted.\n\nMore.\n\n## Acceptance criteria\n",
                1,
            )
            .replace("It works.", "It is edited."),
    );
    let edited = run_review_claims(&root, &head, &[beta, alpha]);
    assert_eq!(
        edited.exit_code,
        std::process::ExitCode::SUCCESS,
        "{}",
        edited.stderr
    );
    assert_eq!(edited.stdout, forward.stdout);
    assert!(!edited.stdout.contains("edited"), "{}", edited.stdout);

    let document: serde_json::Value =
        serde_json::from_str(&forward.stdout).expect("stdout should be valid JSON");
    assert_eq!(document["repository"]["head_sha"], head.as_str());
    let plan_paths: Vec<&str> = document["plans"]
        .as_array()
        .expect("plans should be an array")
        .iter()
        .map(|plan| plan["path"].as_str().expect("path should be a string"))
        .collect();
    assert_eq!(plan_paths, vec![alpha, beta]);
    assert_eq!(
        claim_ids(&document, "claims"),
        vec![format!("{alpha}#AC1"), format!("{beta}#AC1")]
    );

    let alpha_claim = &document["claims"][0];
    let alpha_line = source_line(&alpha_source, "- [ ] AC1:");
    assert_eq!(alpha_claim["source"]["line"], alpha_line);
    assert_eq!(
        alpha_claim["source"]["url"],
        format!("https://github.com/{PR_REPO}/blob/{head}/{alpha}#L{alpha_line}")
    );
    assert_eq!(alpha_claim["statement"], "It works.");
    assert_eq!(
        alpha_claim["tasks"][0]["source"]["line"],
        source_line(&alpha_source, "- [ ] T01:")
    );
    assert_eq!(
        document["claims"][1]["source"]["line"],
        source_line(&beta_source, "- [ ] AC1:")
    );
}

#[test]
fn review_claims_output_is_pure() {
    let (_directory, root) = temp_repository();
    let path = "context/plans/pure.md";
    write_repository_plan(
        &root,
        path,
        &decision_brief_plan_source("pure", &[("Keep one store.", Some("Is it enough?"))]),
    );
    let head = commit_repository(&root);
    let status_before = git_output(&root, &["status", "--porcelain", "--untracked-files=all"]);
    let tree_before = tree_listing(&root);

    let run = run_review_claims(&root, &head, &[path]);

    assert_eq!(
        run.exit_code,
        std::process::ExitCode::SUCCESS,
        "{}",
        run.stderr
    );
    assert!(run.stderr.is_empty(), "{}", run.stderr);
    assert!(run.stdout.ends_with("}\n"), "{:?}", run.stdout);
    assert!(!run.stdout.ends_with("\n\n"), "{:?}", run.stdout);
    let mut values =
        serde_json::Deserializer::from_str(&run.stdout).into_iter::<serde_json::Value>();
    let document = values
        .next()
        .expect("stdout should hold a JSON value")
        .expect("stdout should be valid JSON");
    assert!(values.next().is_none(), "{}", run.stdout);
    assert_eq!(document["schema_version"], "sce.plan.claims.v1");
    assert!(
        run.stdout.starts_with("{\n  \"schema_version\""),
        "{}",
        run.stdout
    );
    assert_eq!(
        git_output(&root, &["status", "--porcelain", "--untracked-files=all"]),
        status_before
    );
    assert_eq!(tree_listing(&root), tree_before);
}

#[cfg(test)]
const CLAIMS_DEFECT_PLANS: [&str; 6] = [
    "context/plans/valid.md",
    "context/plans/invalid.md",
    "context/plans/repeated.md",
    "context/plans/linked.md",
    "context/plans/binary.md",
    "context/plans/corrupt.md",
];

#[cfg(test)]
fn commit_claims_defect_plans(root: &std::path::Path) -> (String, String) {
    use std::io::Write;

    let [valid, invalid, repeated, linked, binary, corrupt] = CLAIMS_DEFECT_PLANS;
    write_repository_plan(
        root,
        valid,
        &decision_brief_plan_source("valid", &[("Keep one store.", Some("Is it enough?"))]),
    );
    write_repository_plan(
        root,
        invalid,
        &std::fs::read_to_string(fixture_dir().join("duplicate-id.md"))
            .expect("fixture should be readable"),
    );
    let repeated_source = repeated_field_source(
        &[],
        &[
            "- [ ] AC1: First.",
            "  - Validate: `true`",
            "  - Scenarios: none (inspection)",
            "- [ ] AC2: Second.",
            "  - Validate: `true`",
            "  - Scenarios: none (inspection)",
        ],
        &["  - Covers: AC1", "  - Covers: AC2"],
    );
    write_repository_plan(root, repeated, &repeated_source);
    std::os::unix::fs::symlink("valid.md", root.join(linked)).expect("symlink should be created");
    let mut bytes = b"# Plan: binary\n\n".to_vec();
    bytes.extend_from_slice(&[0xff, 0xfe, b'\n']);
    std::fs::File::create(root.join(binary))
        .and_then(|mut file| file.write_all(&bytes))
        .expect("binary plan should be written");
    write_repository_plan(
        root,
        corrupt,
        &decision_brief_plan_source("corrupt", &[("Keep a corrupt store.", None)]),
    );
    let head = commit_repository(root);
    let object = git_output(root, &["rev-parse", &format!("{head}:{corrupt}")]);
    std::fs::remove_file(
        root.join(".git/objects")
            .join(&object[..2])
            .join(&object[2..]),
    )
    .expect("loose plan object should be removed");
    (head, repeated_source)
}

#[test]
fn review_claims_rejects_invalid_inputs() {
    let (_directory, root) = temp_repository();
    let (head, repeated_source) = commit_claims_defect_plans(&root);
    let [valid, invalid, repeated, linked, binary, corrupt] = CLAIMS_DEFECT_PLANS;
    let unknown = "0123456789abcdef0123456789abcdef01234567";
    assert_ne!(head, unknown);
    let status_before = git_output(&root, &["status", "--porcelain", "--untracked-files=all"]);

    let run_with = |repo: &str, head: &str, paths: &[&str]| {
        let mut args = vec![NAME, "review", "--claims", "--repo", repo, "--head", head];
        args.extend_from_slice(paths);
        run_plan_args(&args, &root)
    };

    let second_covers = source_line(&repeated_source, "  - Covers: AC2");
    let repeated_stderr = format!(
        "Error [SCE-ERR-VALIDATION]: Plan claims check failed for '{repeated}':\n{repeated}:{second_covers}: `Covers:` appears more than once on `T01`\nTry: merge each repeated field into one line, then rerun 'sce plan review --claims'.\n"
    );
    let invalid_needle = format!("{invalid}:29: `S1` is defined more than once");
    let cases: [(&str, CommandRun, u8, &str); 12] = [
        (
            "invalid repo",
            run_with("not-a-repo", &head, &[valid]),
            3,
            "not-a-repo",
        ),
        (
            "short head",
            run_with(PR_REPO, &head[..12], &[valid]),
            3,
            &head[..12],
        ),
        (
            "uppercase head",
            run_with(PR_REPO, &head.to_uppercase(), &[valid]),
            3,
            "--head",
        ),
        (
            "outside plans",
            run_with(PR_REPO, &head, &["README.md"]),
            3,
            "README.md",
        ),
        (
            "duplicate path",
            run_with(PR_REPO, &head, &[valid, valid]),
            3,
            valid,
        ),
        (
            "invalid plan",
            run_with(PR_REPO, &head, &[valid, invalid]),
            3,
            &invalid_needle,
        ),
        (
            "repeated covers",
            run_with(PR_REPO, &head, &[valid, repeated]),
            3,
            &repeated_stderr,
        ),
        (
            "unknown commit",
            run_with(PR_REPO, unknown, &[valid]),
            4,
            "is not available in the local repository",
        ),
        (
            "absent path",
            run_with(PR_REPO, &head, &[valid, "context/plans/absent.md"]),
            4,
            "does not exist at commit",
        ),
        (
            "symlink",
            run_with(PR_REPO, &head, &[linked]),
            4,
            "is not a regular file (mode 120000",
        ),
        (
            "non-UTF-8",
            run_with(PR_REPO, &head, &[binary]),
            4,
            "is not valid UTF-8",
        ),
        (
            "failed git",
            run_with(PR_REPO, &head, &[valid, corrupt]),
            4,
            "Failed to read plan file",
        ),
    ];
    for (case, run, exit_code, needle) in &cases {
        assert_failure(run, *exit_code, case);
        assert!(run.stderr.contains(needle), "{case}: {}", run.stderr);
    }
    assert_eq!(cases[6].1.stderr, repeated_stderr);
    assert_eq!(
        git_output(&root, &["status", "--porcelain", "--untracked-files=all"]),
        status_before
    );

    assert_repeated_plan_passes_check_and_brief(&root, &head, repeated);
}

#[cfg(test)]
fn assert_repeated_plan_passes_check_and_brief(root: &std::path::Path, head: &str, repeated: &str) {
    let check = run_plan_check(root.join(repeated).to_str().expect("path should be UTF-8"));
    assert_eq!(
        check.exit_code,
        std::process::ExitCode::SUCCESS,
        "{}",
        check.stderr
    );
    let brief = run_review(root, head, &[repeated]);
    assert_eq!(
        brief.exit_code,
        std::process::ExitCode::SUCCESS,
        "{}",
        brief.stderr
    );
}

#[cfg(test)]
const CLAIMS_GOLDEN_REPO: &str = "crocoder-dev/example";

#[cfg(test)]
const CLAIMS_GOLDEN_HEAD: &str = "4f2a9c1e8b7d6053a1c2e3f4a5b6c7d8e9f01234";

#[cfg(test)]
const CLAIMS_GOLDEN_PLAN: &str = r#"# Plan: token-refresh

## Change summary

Add a refresh endpoint.

## Design decisions

- D1: The session store validates refresh tokens.
  - Boundary: `src/auth/` owns token validation.
  - Rationale: The store already tracks revocation.
  - Rejected: Stateless tokens — they cannot be revoked.
  - Trade-offs: One store read per refresh.
  - Review: required
  - Review question: Is one store read per refresh acceptable?

## Acceptance criteria

- [ ] AC1: `POST /auth/refresh` rejects a revoked token with 401.
  - Validate: `pnpm test src/auth/refresh.test.ts`
  - Decisions: D1
  - Scenarios: S1

### Verification scenarios

- S1: Revoked refresh token is rejected
  - Given: A revoked refresh token.
  - When: The client calls `POST /auth/refresh`.
  - Then: The response is 401.
  - Check: `pnpm test src/auth/refresh.test.ts -t "rejects a revoked token"`

## Task stack

- [x] T01: `Add /auth/refresh endpoint` (status:done)
  - Task ID: T01
  - Covers: AC1
  - Scope: In — route handler. Out — rotation policy.
  - Dependencies: none
  - Done when: The endpoint returns 401 for a revoked token.
  - Verify: `pnpm test src/auth/refresh.test.ts`
  - Context synchronization: synced

## Open questions

None.
"#;

#[cfg(test)]
const CLAIMS_GOLDEN_JSON: &str = r#"{
  "schema_version": "sce.plan.claims.v1",
  "repository": {
    "slug": "crocoder-dev/example",
    "head_sha": "4f2a9c1e8b7d6053a1c2e3f4a5b6c7d8e9f01234"
  },
  "plans": [
    {
      "path": "context/plans/token-refresh.md",
      "url": "https://github.com/crocoder-dev/example/blob/4f2a9c1e8b7d6053a1c2e3f4a5b6c7d8e9f01234/context/plans/token-refresh.md"
    }
  ],
  "claims": [
    {
      "id": "context/plans/token-refresh.md#AC1",
      "kind": "acceptance_criterion",
      "statement": "`POST /auth/refresh` rejects a revoked token with 401.",
      "modality": "intended",
      "author_checked": false,
      "source": {
        "path": "context/plans/token-refresh.md",
        "entry_id": "AC1",
        "line": 19,
        "url": "https://github.com/crocoder-dev/example/blob/4f2a9c1e8b7d6053a1c2e3f4a5b6c7d8e9f01234/context/plans/token-refresh.md#L19"
      },
      "decisions": [
        {
          "id": "D1",
          "statement": "The session store validates refresh tokens.",
          "boundary": "`src/auth/` owns token validation.",
          "rationale": "The store already tracks revocation.",
          "rejected": "Stateless tokens — they cannot be revoked.",
          "trade_offs": "One store read per refresh.",
          "review": {
            "required": true,
            "question": "Is one store read per refresh acceptable?"
          },
          "source": {
            "path": "context/plans/token-refresh.md",
            "entry_id": "D1",
            "line": 9,
            "url": "https://github.com/crocoder-dev/example/blob/4f2a9c1e8b7d6053a1c2e3f4a5b6c7d8e9f01234/context/plans/token-refresh.md#L9"
          }
        }
      ],
      "verification": {
        "mode": "scenarios",
        "validate": "`pnpm test src/auth/refresh.test.ts`",
        "scenarios": [
          {
            "id": "S1",
            "statement": "Revoked refresh token is rejected",
            "given": "A revoked refresh token.",
            "when": "The client calls `POST /auth/refresh`.",
            "then": "The response is 401.",
            "check": "`pnpm test src/auth/refresh.test.ts -t \"rejects a revoked token\"`",
            "source": {
              "path": "context/plans/token-refresh.md",
              "entry_id": "S1",
              "line": 26,
              "url": "https://github.com/crocoder-dev/example/blob/4f2a9c1e8b7d6053a1c2e3f4a5b6c7d8e9f01234/context/plans/token-refresh.md#L26"
            }
          }
        ]
      },
      "tasks": [
        {
          "id": "T01",
          "statement": "`Add /auth/refresh endpoint` (status:done)",
          "author_checked": true,
          "source": {
            "path": "context/plans/token-refresh.md",
            "entry_id": "T01",
            "line": 34,
            "url": "https://github.com/crocoder-dev/example/blob/4f2a9c1e8b7d6053a1c2e3f4a5b6c7d8e9f01234/context/plans/token-refresh.md#L34"
          }
        }
      ]
    }
  ]
}"#;

#[cfg(test)]
const CLAIMS_CONTRACT_KEYS: [&str; 33] = [
    "schema_version",
    "repository",
    "plans",
    "claims",
    "slug",
    "head_sha",
    "path",
    "url",
    "id",
    "kind",
    "statement",
    "modality",
    "author_checked",
    "source",
    "decisions",
    "verification",
    "tasks",
    "entry_id",
    "line",
    "boundary",
    "rationale",
    "rejected",
    "trade_offs",
    "review",
    "required",
    "question",
    "mode",
    "validate",
    "scenarios",
    "given",
    "when",
    "then",
    "check",
];

#[cfg(test)]
fn claims_plan_source(lines: &[&str]) -> String {
    let mut source = lines.join("\n");
    source.push('\n');
    source
}

#[cfg(test)]
fn render_claims(
    plans: &[pr_description::PlanInput],
) -> Result<String, crate::services::error::CliError> {
    let paths: Vec<&str> = plans.iter().map(|plan| plan.path.as_str()).collect();
    claims::render(&pr_arguments(&paths), plans)
}

#[cfg(test)]
fn claims_document(plans: &[pr_description::PlanInput]) -> serde_json::Value {
    let rendered = render_claims(plans).expect("claims should render");
    serde_json::from_str(&rendered).expect("claims should be valid JSON")
}

#[cfg(test)]
fn claim_ids<'a>(values: &'a serde_json::Value, key: &str) -> Vec<&'a str> {
    values[key]
        .as_array()
        .expect("value should be an array")
        .iter()
        .map(|value| value["id"].as_str().expect("id should be a string"))
        .collect()
}

#[cfg(test)]
fn collect_keys<'a>(value: &'a serde_json::Value, keys: &mut Vec<&'a str>) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, child) in map {
                keys.push(key);
                collect_keys(child, keys);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_keys(item, keys);
            }
        }
        _ => {}
    }
}

#[test]
fn claims_golden_one_of_each() {
    let path = "context/plans/token-refresh.md";
    let plan = pr_plan_input(path, CLAIMS_GOLDEN_PLAN);
    let arguments = pr_description::PrDescriptionArguments {
        repo: CLAIMS_GOLDEN_REPO.to_string(),
        head: CLAIMS_GOLDEN_HEAD.to_string(),
        paths: vec![path.to_string()],
    };

    let rendered = claims::render(&arguments, &[plan]).expect("claims should render");

    assert_eq!(rendered, CLAIMS_GOLDEN_JSON);
}

#[test]
fn claims_preserve_reference_order() {
    let source = claims_plan_source(&[
        "# Plan: ordering",
        "",
        "## Design decisions",
        "",
        "- D1: First decision.",
        "  - Boundary: Boundary one.",
        "  - Rationale: Rationale one.",
        "  - Rejected: Rejected one.",
        "  - Trade-offs: Trade-offs one.",
        "- D2: Second decision.",
        "  - Boundary: Boundary two.",
        "  - Rationale: Rationale two.",
        "  - Rejected: Rejected two.",
        "  - Trade-offs: Trade-offs two.",
        "",
        "## Acceptance criteria",
        "",
        "- [ ] AC1: Ordered references.",
        "  - Validate: `true`",
        "  - Decisions: D2, D1",
        "  - Scenarios: S2, S1",
        "- [ ] AC2: Other criterion.",
        "  - Validate: `false`",
        "  - Scenarios: none (inspection)",
        "",
        "### Verification scenarios",
        "",
        "- S1: First scenario",
        "  - Given: Given one.",
        "  - When: When one.",
        "  - Then: Then one.",
        "  - Check: Check one.",
        "- S2: Second scenario",
        "  - Given: Given two.",
        "  - When: When two.",
        "  - Then: Then two.",
        "  - Check: Check two.",
        "",
        "## Task stack",
        "",
        "- [ ] T01: `First` (status:todo)",
        "  - Task ID: T01",
        "  - Covers: AC1",
        "- [ ] T02: `Second` (status:todo)",
        "  - Task ID: T02",
        "  - Covers: AC2, AC1",
        "- [ ] T03: `Third` (status:todo)",
        "  - Task ID: T03",
        "  - Covers: AC2",
    ]);
    let document = claims_document(&[pr_plan_input("context/plans/ordering.md", &source)]);
    let first = &document["claims"][0];
    let second = &document["claims"][1];

    assert_eq!(claim_ids(first, "decisions"), vec!["D2", "D1"]);
    assert_eq!(
        claim_ids(&first["verification"], "scenarios"),
        vec!["S2", "S1"]
    );
    assert_eq!(claim_ids(first, "tasks"), vec!["T01", "T02"]);
    assert_eq!(claim_ids(second, "tasks"), vec!["T02", "T03"]);

    let decisions = &first["decisions"];
    assert_eq!(decisions[0]["statement"], "Second decision.");
    assert_eq!(decisions[0]["boundary"], "Boundary two.");
    assert_eq!(decisions[0]["rationale"], "Rationale two.");
    assert_eq!(decisions[0]["rejected"], "Rejected two.");
    assert_eq!(decisions[0]["trade_offs"], "Trade-offs two.");
    assert_eq!(decisions[0]["source"]["line"], 10);
    assert_eq!(decisions[1]["statement"], "First decision.");
    assert_eq!(decisions[1]["boundary"], "Boundary one.");
    assert_eq!(decisions[1]["source"]["line"], 5);

    let scenarios = &first["verification"]["scenarios"];
    assert_eq!(scenarios[0]["statement"], "Second scenario");
    assert_eq!(scenarios[0]["given"], "Given two.");
    assert_eq!(scenarios[0]["when"], "When two.");
    assert_eq!(scenarios[0]["then"], "Then two.");
    assert_eq!(scenarios[0]["check"], "Check two.");
    assert_eq!(scenarios[0]["source"]["line"], 33);
    assert_eq!(scenarios[1]["statement"], "First scenario");
    assert_eq!(scenarios[1]["check"], "Check one.");
    assert_eq!(scenarios[1]["source"]["line"], 28);

    let tasks = &first["tasks"];
    assert_eq!(tasks[0]["source"]["line"], 41);
    assert_eq!(tasks[1]["source"]["line"], 44);
    assert_eq!(second["source"]["line"], 22);
    assert_eq!(second["id"], "context/plans/ordering.md#AC2");
}

#[test]
fn claims_inspection_only_criterion() {
    let source = review_plan_source("inspection", [None, None, None]);
    let document = claims_document(&[pr_plan_input("context/plans/inspection.md", &source)]);

    assert_eq!(
        document["claims"][0]["verification"],
        serde_json::json!({
            "mode": "inspection",
            "validate": "`true`",
            "scenarios": []
        })
    );
}

#[test]
fn claims_unmarked_and_absent_decisions() {
    let source = claims_plan_source(&[
        "# Plan: unmarked",
        "",
        "## Design decisions",
        "",
        "- D1: Unmarked decision.",
        "  - Boundary: One module.",
        "  - Rationale: It is simple.",
        "  - Rejected: Two modules.",
        "  - Trade-offs: None.",
        "- D2: Marked decision.",
        "  - Boundary: One module.",
        "  - Rationale: It is simple.",
        "  - Rejected: Two modules.",
        "  - Trade-offs: None.",
        "  - Review: required",
        "  - Review question: Is one module enough?",
        "",
        "## Acceptance criteria",
        "",
        "- [ ] AC1: References an unmarked decision.",
        "  - Validate: `true`",
        "  - Decisions: D1",
        "  - Scenarios: none (inspection)",
        "- [ ] AC2: References no decision.",
        "  - Validate: `true`",
        "  - Scenarios: none (inspection)",
        "- [ ] AC3: References a marked decision.",
        "  - Validate: `true`",
        "  - Decisions: D2",
        "  - Scenarios: none (inspection)",
        "",
        "## Task stack",
        "",
        "- [ ] T01: `Do it` (status:todo)",
        "  - Task ID: T01",
        "  - Covers: AC1, AC2, AC3",
    ]);
    let document = claims_document(&[pr_plan_input("context/plans/unmarked.md", &source)]);
    let claims = &document["claims"];

    assert_eq!(
        claims[0]["decisions"][0]["review"],
        serde_json::json!({"required": false, "question": null})
    );
    assert_eq!(claims[1]["decisions"], serde_json::json!([]));
    assert_eq!(
        claims[2]["decisions"][0]["review"],
        serde_json::json!({"required": true, "question": "Is one module enough?"})
    );
}

#[test]
fn claims_checked_entries_stay_intended() {
    let source = claims_plan_source(&[
        "# Plan: checked",
        "",
        "## Acceptance criteria",
        "",
        "- [x] AC1: Checked criterion keeps *stars* and snake_case.",
        "  - Validate: `true`",
        "  - Scenarios: none (inspection)",
        "- [ ] AC2: Unchecked criterion.",
        "  - Validate: `true`",
        "  - Scenarios: none (inspection)",
        "- AC3: Criterion without a checkbox.",
        "  - Validate: `true`",
        "  - Scenarios: none (inspection)",
        "",
        "## Task stack",
        "",
        "- [x] T01: `Done task` (status:done)",
        "  - Task ID: T01",
        "  - Covers: AC1, AC2, AC3",
        "- [ ] T02: `Open task` (status:todo)",
        "  - Task ID: T02",
        "  - Covers: AC1",
        "- T03: `Plain task`",
        "  - Task ID: T03",
        "  - Covers: AC1",
    ]);
    let document = claims_document(&[pr_plan_input("context/plans/checked.md", &source)]);
    let claims = document["claims"]
        .as_array()
        .expect("claims should be an array");

    let checked: Vec<&serde_json::Value> = claims
        .iter()
        .map(|claim| &claim["author_checked"])
        .collect();
    assert_eq!(
        checked,
        vec![
            &serde_json::json!(true),
            &serde_json::json!(false),
            &serde_json::Value::Null
        ]
    );
    let task_checked: Vec<&serde_json::Value> = claims[0]["tasks"]
        .as_array()
        .expect("tasks should be an array")
        .iter()
        .map(|task| &task["author_checked"])
        .collect();
    assert_eq!(
        task_checked,
        vec![
            &serde_json::json!(true),
            &serde_json::json!(false),
            &serde_json::Value::Null
        ]
    );
    assert_eq!(
        claims[0]["statement"],
        "Checked criterion keeps *stars* and snake_case."
    );
    assert_eq!(
        claims[0]["tasks"][0]["statement"],
        "`Done task` (status:done)"
    );
    for claim in claims {
        assert_eq!(claim["kind"], "acceptance_criterion");
        assert_eq!(claim["modality"], "intended");
    }

    let mut keys = Vec::new();
    collect_keys(&document, &mut keys);
    for key in keys {
        assert!(
            CLAIMS_CONTRACT_KEYS.contains(&key),
            "unexpected key `{key}`"
        );
    }
}

#[test]
fn claims_include_every_criterion() {
    use std::fmt::Write;

    let mut source = decision_brief_plan_source(
        "unlimited",
        &[
            ("First.", Some("Question one?")),
            ("Second.", Some("Question two?")),
            ("", Some("Question three?")),
            ("Fourth.", Some("Question four?")),
        ],
    );
    let criteria_start = source
        .find("\n## Acceptance criteria")
        .expect("source should hold acceptance criteria");
    source.truncate(criteria_start);
    source.push_str("\n## Acceptance criteria\n\n");
    for index in 1..=5 {
        write!(
            source,
            "- [ ] AC{index}: Criterion {index}.\n  - Validate: `true`\n  - Decisions: D1, D2, D3, D4\n  - Scenarios: none (inspection)\n"
        )
        .expect("writing to String should not fail");
    }
    source.push_str(
        "\n## Task stack\n\n- [ ] T01: `Do it` (status:todo)\n  - Task ID: T01\n  - Covers: AC1, AC2, AC3, AC4, AC5\n",
    );
    let unlimited = pr_plan_input("context/plans/unlimited.md", &source);
    let document = claims_document(std::slice::from_ref(&unlimited));
    let ids: Vec<&str> = document["claims"]
        .as_array()
        .expect("claims should be an array")
        .iter()
        .map(|claim| claim["id"].as_str().expect("id should be a string"))
        .collect();
    assert_eq!(
        ids,
        vec![
            "context/plans/unlimited.md#AC1",
            "context/plans/unlimited.md#AC2",
            "context/plans/unlimited.md#AC3",
            "context/plans/unlimited.md#AC4",
            "context/plans/unlimited.md#AC5",
        ]
    );
    assert_eq!(document["claims"][0]["decisions"][2]["statement"], "");
    assert_eq!(
        claim_ids(&document["claims"][4], "decisions"),
        vec!["D1", "D2", "D3", "D4"]
    );

    let empty = claims_plan_source(&[
        "# Plan: empty",
        "",
        "## Acceptance criteria",
        "",
        "None.",
        "",
        "## Task stack",
        "",
        "None.",
    ]);
    let empty = pr_plan_input("context/plans/empty.md", &empty);
    let document = claims_document(std::slice::from_ref(&empty));
    assert_eq!(document["claims"], serde_json::json!([]));
    assert_eq!(document["plans"][0]["path"], "context/plans/empty.md");

    let both = claims_document(&[unlimited, empty]);
    assert_eq!(
        both["plans"]
            .as_array()
            .expect("plans should be an array")
            .iter()
            .map(|plan| plan["path"].as_str().expect("path should be a string"))
            .collect::<Vec<_>>(),
        vec!["context/plans/empty.md", "context/plans/unlimited.md"]
    );
    assert_eq!(both["claims"].as_array().map(Vec::len), Some(5));
}

#[cfg(test)]
fn repeated_field_source(decision_extra: &[&str], criteria: &[&str], covers: &[&str]) -> String {
    let mut lines = vec![
        "# Plan: repeated",
        "",
        "## Design decisions",
        "",
        "- D1: One decision.",
        "  - Boundary: One module.",
        "  - Rationale: It is simple.",
        "  - Rejected: Two modules.",
        "  - Trade-offs: None.",
    ];
    lines.extend(decision_extra);
    lines.extend(["", "## Acceptance criteria", ""]);
    lines.extend(criteria);
    lines.extend([
        "",
        "## Task stack",
        "",
        "- [ ] T01: Implement the feature",
        "  - Task ID: T01",
    ]);
    lines.extend(covers);
    claims_plan_source(&lines)
}

#[test]
fn claims_reject_repeated_consumed_fields() {
    let covers = repeated_field_source(
        &[],
        &[
            "- [ ] AC1: First.",
            "  - Validate: `true`",
            "  - Scenarios: none (inspection)",
            "- [ ] AC2: Second.",
            "  - Validate: `true`",
            "  - Scenarios: none (inspection)",
        ],
        &["  - Covers: AC1", "  - Covers: AC2"],
    );
    let scenarios = repeated_field_source(
        &[],
        &[
            "- [ ] AC1: First.",
            "  - Validate: `true`",
            "  - Scenarios: S1",
            "  - Scenarios: S2",
            "",
            "### Verification scenarios",
            "",
            "- S1: One",
            "  - Given: G.",
            "  - When: W.",
            "  - Then: T.",
            "  - Check: C.",
            "- S2: Two",
            "  - Given: G.",
            "  - When: W.",
            "  - Then: T.",
            "  - Check: C.",
        ],
        &["  - Covers: AC1"],
    );
    let decisions = repeated_field_source(
        &[],
        &[
            "- [ ] AC1: First.",
            "  - Validate: `true`",
            "  - Decisions: D1",
            "  - Decisions: D1",
            "  - Scenarios: none (inspection)",
        ],
        &["  - Covers: AC1"],
    );
    let rationale = repeated_field_source(
        &["  - Rationale: It is also cheap."],
        &[
            "- [ ] AC1: First.",
            "  - Validate: `true`",
            "  - Decisions: D1",
            "  - Scenarios: none (inspection)",
        ],
        &["  - Covers: AC1"],
    );

    for (source, expected) in [
        (
            &covers,
            "context/plans/repeated.md:25: `Covers:` appears more than once on `T01`",
        ),
        (
            &scenarios,
            "context/plans/repeated.md:16: `Scenarios:` appears more than once on `AC1`",
        ),
        (
            &decisions,
            "context/plans/repeated.md:16: `Decisions:` appears more than once on `AC1`",
        ),
        (
            &rationale,
            "context/plans/repeated.md:10: `Rationale:` appears more than once on `D1`",
        ),
    ] {
        let plan = pr_plan_input("context/plans/repeated.md", source);
        let error = render_claims(std::slice::from_ref(&plan))
            .expect_err("repeated consumed field should fail claims");

        assert_eq!(error.class().exit_code(), 3);
        assert_eq!(
            error.to_string(),
            format!(
                "Plan claims check failed for 'context/plans/repeated.md':\n{expected}\nTry: merge each repeated field into one line, then rerun 'sce plan review --claims'."
            )
        );
    }
}
