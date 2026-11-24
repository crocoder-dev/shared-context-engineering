use std::cell::RefCell;
use std::collections::HashMap;
use std::process::Command;

use super::*;

fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_string()).collect()
}

fn telemetry(directory: Option<&str>) -> LaunchMode {
    LaunchMode::Telemetry {
        directory: directory.map(PathBuf::from),
    }
}

fn plain(reason: PlainLaunchReason) -> LaunchMode {
    LaunchMode::Plain(reason)
}

#[test]
fn codex_telemetry_classifies_new_sessions_exec_and_exec_fork_as_telemetry_modes() {
    for (argv, expected) in [
        (vec![], telemetry(None)),
        (vec!["fix the failing test"], telemetry(None)),
        (
            vec!["-m", "gpt-5", "--full-auto", "prompt"],
            telemetry(None),
        ),
        (vec!["exec", "prompt"], telemetry(None)),
        (vec!["e", "--json", "prompt"], telemetry(None)),
        (vec!["exec"], telemetry(None)),
        (vec!["exec", "fork", "019a-session"], telemetry(None)),
        (
            vec!["-C", "../other", "exec", "prompt"],
            telemetry(Some("../other")),
        ),
        (vec!["--cd=/work/b"], telemetry(Some("/work/b"))),
        (vec!["-C/work/b", "prompt"], telemetry(Some("/work/b"))),
        (vec!["-m", "resume"], telemetry(None)),
        (vec!["-c", "model=fork", "prompt"], telemetry(None)),
        (vec!["--", "--help"], telemetry(None)),
    ] {
        assert_eq!(classify_launch(&args(&argv)), expected, "{argv:?}");
    }
}

#[test]
fn codex_telemetry_requires_explicit_directory_for_resume() {
    for (argv, expected) in [
        (
            vec!["resume", "019a-session"],
            plain(PlainLaunchReason::ResumeWithoutDirectory),
        ),
        (
            vec!["resume", "--last"],
            plain(PlainLaunchReason::ResumeWithoutDirectory),
        ),
        (
            vec!["exec", "resume", "019a-session", "prompt"],
            plain(PlainLaunchReason::ResumeWithoutDirectory),
        ),
        (
            vec!["resume", "019a-session", "-C", "/work/a"],
            telemetry(Some("/work/a")),
        ),
        (
            vec!["-C", "/work/a", "resume", "019a-session"],
            telemetry(Some("/work/a")),
        ),
        (
            vec!["exec", "resume", "--cd", "/work/a", "019a-session"],
            telemetry(Some("/work/a")),
        ),
    ] {
        assert_eq!(classify_launch(&args(&argv)), expected, "{argv:?}");
    }
}

#[test]
fn codex_telemetry_launches_unproven_and_sessionless_modes_without_telemetry() {
    for (argv, expected) in [
        (
            vec!["fork", "019a-session"],
            plain(PlainLaunchReason::InteractiveFork),
        ),
        (
            vec!["-c", "model=o3", "fork"],
            plain(PlainLaunchReason::InteractiveFork),
        ),
        (
            vec!["app-server"],
            plain(PlainLaunchReason::UnsupportedSubcommand(
                "app-server".into(),
            )),
        ),
        (
            vec!["exec", "review"],
            plain(PlainLaunchReason::UnsupportedSubcommand(
                "exec review".into(),
            )),
        ),
        (
            vec!["mcp", "list"],
            plain(PlainLaunchReason::UnsupportedSubcommand("mcp".into())),
        ),
        (
            vec!["--brand-new-flag", "resume"],
            plain(PlainLaunchReason::UnrecognizedOption(
                "--brand-new-flag".into(),
            )),
        ),
        (
            vec!["--brand-new=value"],
            plain(PlainLaunchReason::UnrecognizedOption(
                "--brand-new=value".into(),
            )),
        ),
        (vec!["login"], plain(PlainLaunchReason::Sessionless)),
        (vec!["--help"], plain(PlainLaunchReason::Sessionless)),
        (vec!["exec", "-h"], plain(PlainLaunchReason::Sessionless)),
        (vec!["--version"], plain(PlainLaunchReason::Sessionless)),
    ] {
        assert_eq!(classify_launch(&args(&argv)), expected, "{argv:?}");
    }
}

struct ConfigFiles(HashMap<PathBuf, io::Result<String>>);

impl ConfigFiles {
    fn new(entries: Vec<(&str, io::Result<&str>)>) -> Self {
        Self(
            entries
                .into_iter()
                .map(|(path, contents)| (PathBuf::from(path), contents.map(str::to_string)))
                .collect(),
        )
    }

    fn read(&self, path: &Path) -> io::Result<Option<String>> {
        match self.0.get(path) {
            None => Ok(None),
            Some(Ok(contents)) => Ok(Some(contents.clone())),
            Some(Err(error)) => Err(io::Error::new(error.kind(), "denied")),
        }
    }
}

fn no_environment(_: &str) -> Option<String> {
    None
}

fn conflict_for(
    argv: &[&str],
    files: &ConfigFiles,
    environment: impl Fn(&str) -> Option<String>,
) -> Option<OtelConflict> {
    detect_otel_conflict(
        &args(argv),
        Some(Path::new("/home/u/.codex")),
        |path| files.read(path),
        environment,
    )
}

fn denied() -> io::Result<&'static str> {
    Err(io::Error::from(io::ErrorKind::PermissionDenied))
}

type ConflictCase = (
    &'static str,
    Vec<&'static str>,
    Vec<(&'static str, io::Result<&'static str>)>,
    OtelConflict,
);

const USER_CONFIG: &str = "/home/u/.codex/config.toml";

fn assert_conflicts(cases: Vec<ConflictCase>) {
    for (case, argv, files, expected) in cases {
        assert_eq!(
            conflict_for(&argv, &ConfigFiles::new(files), no_environment),
            Some(expected),
            "{case}"
        );
    }
}

#[test]
fn codex_telemetry_conflict_detection_skips_for_caller_user_and_profile_otel() {
    let user = USER_CONFIG;
    assert_conflicts(vec![
        (
            "caller override",
            vec!["-c", "otel.trace_exporter=none"],
            vec![],
            OtelConflict::CallerOverride,
        ),
        (
            "caller override attached",
            vec!["--config=otel.environment=\"prod\"", "exec"],
            vec![],
            OtelConflict::CallerOverride,
        ),
        (
            "caller otel table",
            vec!["-cotel={exporter=\"none\"}"],
            vec![],
            OtelConflict::CallerOverride,
        ),
        (
            "user exporter",
            vec![],
            vec![(user, Ok("[otel]\nexporter = { otlp-http = { endpoint = \"https://mine/v1/logs\", protocol = \"json\" } }\n"))],
            OtelConflict::UserConfig(PathBuf::from(user)),
        ),
        (
            "user helper",
            vec![],
            vec![(user, Ok("otel.headers_helper = \"my-helper\"\n"))],
            OtelConflict::UserConfig(PathBuf::from(user)),
        ),
        (
            "profile file from argument",
            vec!["--profile", "work"],
            vec![(
                "/home/u/.codex/work.config.toml",
                Ok("[otel]\nmetrics_exporter = \"statsig\"\n"),
            )],
            OtelConflict::ProfileConfig(PathBuf::from("/home/u/.codex/work.config.toml")),
        ),
        (
            "default inline profile",
            vec![],
            vec![(
                user,
                Ok("profile = \"work\"\n[profiles.work.otel]\ntrace_exporter = \"none\"\n"),
            )],
            OtelConflict::ProfileConfig(PathBuf::from(user)),
        ),
        (
            "unreadable user config",
            vec![],
            vec![(user, denied())],
            OtelConflict::UnreadableConfig(PathBuf::from(user)),
        ),
        (
            "invalid user config",
            vec![],
            vec![(user, Ok("otel = [unterminated"))],
            OtelConflict::UnreadableConfig(PathBuf::from(user)),
        ),
        (
            "invalid profile file",
            vec!["-p", "work"],
            vec![("/home/u/.codex/work.config.toml", Ok("[[["))],
            OtelConflict::UnreadableConfig(PathBuf::from("/home/u/.codex/work.config.toml")),
        ),
    ]);
}

#[test]
fn codex_telemetry_conflict_detection_skips_for_managed_system_otel() {
    assert_conflicts(vec![
        (
            "requirements",
            vec![],
            vec![(
                "/etc/codex/requirements.toml",
                Ok("[otel]\nlog_user_prompt = true\n"),
            )],
            OtelConflict::SystemConfig(PathBuf::from("/etc/codex/requirements.toml")),
        ),
        (
            "managed config",
            vec![],
            vec![(
                "/etc/codex/managed_config.toml",
                Ok("otel.environment = \"corp\"\n"),
            )],
            OtelConflict::SystemConfig(PathBuf::from("/etc/codex/managed_config.toml")),
        ),
        (
            "system config",
            vec![],
            vec![("/etc/codex/config.toml", Ok("[otel]\n"))],
            OtelConflict::SystemConfig(PathBuf::from("/etc/codex/config.toml")),
        ),
    ]);
}

#[test]
fn codex_telemetry_conflict_detection_allows_unrelated_settings() {
    let files = ConfigFiles::new(vec![
        (
            "/home/u/.codex/config.toml",
            Ok("model = \"o3\"\nprofile = \"work\"\n[otel]\nenvironment = \"dev\"\nlog_user_prompt = false\n[profiles.work]\nmodel = \"gpt-5\"\n"),
        ),
        ("/home/u/.codex/work.config.toml", Ok("model = \"gpt-5\"\n")),
        ("/etc/codex/config.toml", Ok("model = \"o3\"\n")),
        ("/etc/codex/requirements.toml", denied()),
    ]);
    assert_eq!(
        conflict_for(
            &["-c", "model=o3", "-c", "hotel.x=1", "exec"],
            &files,
            no_environment
        ),
        None
    );
    assert_eq!(
        conflict_for(&[], &ConfigFiles::new(vec![]), no_environment),
        None
    );
}

#[test]
fn codex_telemetry_conflict_detection_refuses_foreign_destination_environment_and_unknown_home() {
    let foreign = |key: &str| {
        (key == "OTEL_EXPORTER_OTLP_ENDPOINT").then(|| "http://localhost:4318".to_string())
    };
    assert_eq!(
        conflict_for(&[], &ConfigFiles::new(vec![]), foreign),
        Some(OtelConflict::DestinationEnvironment)
    );
    let managed = |key: &str| {
        (key == "OTEL_EXPORTER_OTLP_ENDPOINT").then(|| "https://otel.crocoderlab.dev".to_string())
    };
    assert_eq!(conflict_for(&[], &ConfigFiles::new(vec![]), managed), None);
    assert_eq!(
        detect_otel_conflict(&[], None, |_| Ok(None), no_environment),
        Some(OtelConflict::UnknownCodexHome)
    );
}

fn overrides_as_toml(overrides: &[String]) -> toml::Table {
    assert_eq!(overrides.len() % 2, 0);
    let document = overrides
        .chunks(2)
        .map(|pair| {
            assert_eq!(pair[0], "-c");
            let (key, value) = pair[1].split_once('=').expect("key=value override");
            format!("{key} = {value}")
        })
        .collect::<Vec<_>>()
        .join("\n");
    document
        .parse::<toml::Table>()
        .expect("overrides are valid TOML")
}

#[test]
fn codex_telemetry_overrides_pin_every_exporter_and_disable_content() {
    let overrides = telemetry_overrides(Path::new("/opt/sce/bin/sce"), Path::new("/work/repo"));
    let parsed = overrides_as_toml(&overrides);
    let otel = parsed["otel"].as_table().expect("otel table");

    assert_eq!(otel["exporter"].as_str(), Some("none"));
    for (key, endpoint) in [
        ("trace_exporter", "https://otel.crocoderlab.dev/v1/traces"),
        (
            "metrics_exporter",
            "https://otel.crocoderlab.dev/v1/metrics",
        ),
    ] {
        let exporter = otel[key]["otlp-http"].as_table().expect(key);
        assert_eq!(exporter["endpoint"].as_str(), Some(endpoint), "{key}");
        assert_eq!(exporter["protocol"].as_str(), Some("binary"), "{key}");
        assert!(
            !exporter.contains_key("headers"),
            "{key} has static headers"
        );
    }
    assert_eq!(
        otel["headers_helper_refresh_interval_ms"].as_integer(),
        Some(300_000)
    );
    for key in [
        "log_user_prompt",
        "log_agent_responses",
        "log_guardian_assessments",
    ] {
        assert_eq!(otel[key].as_bool(), Some(false), "{key}");
    }
    for override_value in &overrides {
        assert!(!override_value.contains("Bearer"), "{override_value}");
        assert!(
            !override_value.contains("authorization"),
            "{override_value}"
        );
    }
}

#[test]
fn codex_telemetry_helper_command_round_trips_through_the_shell() {
    let repository = "/work/it's a $HOME `repo`\\ \"x\"";
    let executable = "/opt/s ce/sce";
    let overrides = telemetry_overrides(Path::new(executable), Path::new(repository));
    let parsed = overrides_as_toml(&overrides);
    let helper = parsed["otel"]["headers_helper"]
        .as_str()
        .expect("helper command");

    let output = Command::new("sh")
        .arg("-c")
        .arg(format!("set -- {helper}; printf '%s\\0' \"$@\""))
        .output()
        .expect("run sh");
    assert!(output.status.success());
    let words = String::from_utf8(output.stdout).unwrap();
    let words = words.trim_end_matches('\0').split('\0').collect::<Vec<_>>();
    assert_eq!(
        words,
        [
            executable,
            "auth",
            "telemetry-headers",
            "--repo",
            repository
        ]
    );
}

struct Repositories {
    _dir: tempfile::TempDir,
    root: PathBuf,
    first: PathBuf,
    second: PathBuf,
    plain: PathBuf,
}

fn repositories() -> Repositories {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let first = root.join("first");
    let second = root.join("second");
    let plain = root.join("plain");
    for repository in [&first, &second] {
        std::fs::create_dir_all(repository.join(".git")).unwrap();
        std::fs::create_dir_all(repository.join("src")).unwrap();
    }
    std::fs::create_dir_all(&plain).unwrap();
    Repositories {
        _dir: dir,
        root,
        first,
        second,
        plain,
    }
}

fn context_in(working_dir: &Path) -> LaunchContext {
    LaunchContext {
        working_dir: working_dir.to_path_buf(),
        codex_home: Some(PathBuf::from("/home/u/.codex")),
        sce_executable: Some(PathBuf::from("/opt/sce/bin/sce")),
    }
}

fn repository_root_within(sandbox: &Path) -> impl FnOnce(&Path) -> Option<PathBuf> + '_ {
    move |launch_dir| repository_root_for(launch_dir).filter(|root| root.starts_with(sandbox))
}

struct Outcome {
    plan: LaunchPlan,
    policy_checked: Option<PathBuf>,
    authorized_for: Option<PathBuf>,
}

fn plan(
    argv: &[&str],
    sandbox: &Path,
    context: &LaunchContext,
    files: &ConfigFiles,
    telemetry_enabled: bool,
    authorized: bool,
) -> Outcome {
    let policy_checked = RefCell::new(None);
    let authorized_for = RefCell::new(None);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let plan = runtime.block_on(plan_launch_with(
        args(argv),
        context,
        |path| files.read(path),
        no_environment,
        repository_root_within(sandbox),
        |root: &Path| {
            *policy_checked.borrow_mut() = Some(root.to_path_buf());
            telemetry_enabled
        },
        |root: PathBuf| {
            *authorized_for.borrow_mut() = Some(root);
            async move { authorized }
        },
    ));
    Outcome {
        plan,
        policy_checked: policy_checked.into_inner(),
        authorized_for: authorized_for.into_inner(),
    }
}

fn helper_repository(argv: &[String]) -> String {
    let parsed = overrides_as_toml(&argv[..16]);
    parsed["otel"]["headers_helper"]
        .as_str()
        .unwrap()
        .rsplit_once(" --repo ")
        .unwrap()
        .1
        .trim_matches('\'')
        .to_string()
}

#[test]
fn codex_telemetry_plan_prepends_overrides_for_the_resolved_repository() {
    let repos = repositories();
    let files = ConfigFiles::new(vec![]);

    let outcome = plan(
        &["exec", "--json", "prompt"],
        &repos.root,
        &context_in(&repos.first.join("src")),
        &files,
        true,
        true,
    );
    assert_eq!(outcome.plan.notice, None);
    assert_eq!(
        outcome.policy_checked.as_deref(),
        Some(repos.first.as_path())
    );
    assert_eq!(
        outcome.authorized_for.as_deref(),
        Some(repos.first.as_path())
    );
    assert_eq!(outcome.plan.argv[16..], args(&["exec", "--json", "prompt"]));
    assert_eq!(
        helper_repository(&outcome.plan.argv),
        repos.first.to_string_lossy()
    );

    let resumed = plan(
        &["resume", "019a-session", "-C", "../second"],
        &repos.root,
        &context_in(&repos.first),
        &files,
        true,
        true,
    );
    assert_eq!(resumed.plan.notice, None);
    assert_eq!(
        resumed.authorized_for.as_deref(),
        Some(repos.second.as_path())
    );
    assert_eq!(
        helper_repository(&resumed.plan.argv),
        repos.second.to_string_lossy()
    );
}

#[test]
fn codex_telemetry_plan_forwards_arguments_unchanged_when_telemetry_is_skipped() {
    let repos = repositories();
    let conflicting = ConfigFiles::new(vec![(
        "/home/u/.codex/config.toml",
        Ok("[otel]\ntrace_exporter = \"none\"\n"),
    )]);
    let clean = ConfigFiles::new(vec![]);
    let argv = ["-m", "o3", "exec", "prompt"];

    let cases = [
        (
            "not a repository",
            context_in(&repos.plain),
            &clean,
            true,
            true,
            true,
            false,
        ),
        (
            "policy disabled",
            context_in(&repos.first),
            &clean,
            false,
            true,
            false,
            false,
        ),
        (
            "conflict",
            context_in(&repos.first),
            &conflicting,
            true,
            true,
            true,
            false,
        ),
        (
            "unauthorized",
            context_in(&repos.first),
            &clean,
            true,
            false,
            true,
            true,
        ),
    ];
    for (case, context, files, enabled, authorized, expect_notice, expect_lookup) in cases {
        let outcome = plan(&argv, &repos.root, &context, files, enabled, authorized);
        assert_eq!(outcome.plan.argv, args(&argv), "{case}");
        assert_eq!(outcome.plan.notice.is_some(), expect_notice, "{case}");
        assert_eq!(outcome.authorized_for.is_some(), expect_lookup, "{case}");
    }

    let conflict = plan(
        &argv,
        &repos.root,
        &context_in(&repos.first),
        &conflicting,
        true,
        true,
    );
    assert_eq!(
        conflict.plan.notice.as_deref(),
        Some("sce: launching Codex without SCE native telemetry: /home/u/.codex/config.toml defines a user-owned OTEL exporter.")
    );

    let resume = plan(
        &["resume", "--last"],
        &repos.root,
        &context_in(&repos.first),
        &clean,
        true,
        true,
    );
    assert_eq!(resume.plan.argv, args(&["resume", "--last"]));
    assert!(resume.plan.notice.is_some());
    assert_eq!(resume.policy_checked, None);

    let help = plan(
        &["--help"],
        &repos.root,
        &context_in(&repos.first),
        &clean,
        true,
        true,
    );
    assert_eq!(
        help.plan,
        LaunchPlan {
            argv: args(&["--help"]),
            notice: None
        }
    );
}
