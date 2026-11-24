pub mod command;

use std::fmt;
use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};

use crate::services::default_paths::enclosing_repository_root;
use crate::services::managed_telemetry::claude_helper::visible_destination_conflict;
use crate::services::managed_telemetry::{
    MANAGED_OTLP_METRICS_ENDPOINT, MANAGED_OTLP_TRACES_ENDPOINT,
};

pub const NAME: &str = "codex";

const CODEX_PROGRAM: &str = "codex";
const CODEX_HOME_ENV: &str = "CODEX_HOME";
const HEADERS_HELPER_REFRESH_INTERVAL_MS: u32 = 300_000;
const OTLP_HTTP_BINARY_PROTOCOL: &str = "binary";
const NOTICE_PREFIX: &str = "sce: launching Codex without SCE native telemetry";

const SYSTEM_CONFIG_PATHS: [&str; 3] = [
    "/etc/codex/requirements.toml",
    "/etc/codex/managed_config.toml",
    "/etc/codex/config.toml",
];
const USER_OWNED_OTEL_KEYS: [&str; 4] = [
    "exporter",
    "trace_exporter",
    "metrics_exporter",
    "headers_helper",
];

const CONFIG_OPTIONS: [&str; 2] = ["-c", "--config"];
const PROFILE_OPTIONS: [&str; 2] = ["-p", "--profile"];
const DIRECTORY_OPTIONS: [&str; 2] = ["-C", "--cd"];
const OTHER_VALUE_OPTIONS: [&str; 16] = [
    "-m",
    "--model",
    "-s",
    "--sandbox",
    "-a",
    "--ask-for-approval",
    "-i",
    "--image",
    "--enable",
    "--disable",
    "--local-provider",
    "--add-dir",
    "--output-schema",
    "-o",
    "--output-last-message",
    "--color",
];
const FLAG_OPTIONS: [&str; 10] = [
    "--oss",
    "--search",
    "--full-auto",
    "--dangerously-bypass-approvals-and-sandbox",
    "--yolo",
    "--skip-git-repo-check",
    "--json",
    "--ephemeral",
    "--no-alt-screen",
    "--dangerously-bypass-hook-trust",
];
const HELP_OR_VERSION_OPTIONS: [&str; 4] = ["-h", "--help", "-V", "--version"];

const EXEC_SUBCOMMANDS: [&str; 2] = ["exec", "e"];
const RESUME_SUBCOMMAND: &str = "resume";
const FORK_SUBCOMMAND: &str = "fork";
const SESSIONLESS_SUBCOMMANDS: [&str; 9] = [
    "login",
    "logout",
    "completion",
    "features",
    "apply",
    "a",
    "sandbox",
    "debug",
    "help",
];
const UNSUPPORTED_SUBCOMMANDS: [&str; 7] = [
    "app-server",
    "exec-server",
    "remote-control",
    "cloud",
    "mcp",
    "mcp-server",
    "review",
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum LaunchMode {
    Telemetry { directory: Option<PathBuf> },
    Plain(PlainLaunchReason),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PlainLaunchReason {
    Sessionless,
    ResumeWithoutDirectory,
    InteractiveFork,
    UnsupportedSubcommand(String),
    UnrecognizedOption(String),
    NotRepository,
    TelemetryDisabled,
    Conflict(OtelConflict),
    CredentialUnavailable,
    ExecutableUnavailable,
}

impl PlainLaunchReason {
    fn notice(&self) -> Option<String> {
        let reason = match self {
            Self::Sessionless | Self::TelemetryDisabled => return None,
            Self::ResumeWithoutDirectory => {
                "resume needs an explicit `-C <dir>` to attribute the session to a repository"
                    .to_string()
            }
            Self::InteractiveFork => {
                "interactive fork chooses its working directory after launch".to_string()
            }
            Self::UnsupportedSubcommand(subcommand) => {
                format!("`{subcommand}` is not a supported telemetry launch mode")
            }
            Self::UnrecognizedOption(option) => {
                format!("unrecognized Codex option `{option}`")
            }
            Self::NotRepository => {
                "the launch directory is not inside a Git repository".to_string()
            }
            Self::Conflict(conflict) => conflict.to_string(),
            Self::CredentialUnavailable => {
                "no telemetry credential is authorized for this repository".to_string()
            }
            Self::ExecutableUnavailable => {
                "the sce executable path could not be determined".to_string()
            }
        };
        Some(format!("{NOTICE_PREFIX}: {reason}."))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum OtelConflict {
    DestinationEnvironment,
    CallerOverride,
    UnknownCodexHome,
    UserConfig(PathBuf),
    ProfileConfig(PathBuf),
    SystemConfig(PathBuf),
    UnreadableConfig(PathBuf),
}

impl fmt::Display for OtelConflict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DestinationEnvironment => {
                write!(
                    f,
                    "OTEL_EXPORTER_OTLP_* environment variables name another destination"
                )
            }
            Self::CallerOverride => write!(f, "the launch arguments override `otel.*` settings"),
            Self::UnknownCodexHome => write!(f, "the Codex home directory could not be determined"),
            Self::UserConfig(path) => {
                write!(f, "{} defines a user-owned OTEL exporter", path.display())
            }
            Self::ProfileConfig(path) => {
                write!(
                    f,
                    "the selected profile in {} defines OTEL settings",
                    path.display()
                )
            }
            Self::SystemConfig(path) => {
                write!(f, "{} defines managed OTEL settings", path.display())
            }
            Self::UnreadableConfig(path) => {
                write!(f, "{} could not be read as Codex config", path.display())
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LaunchPlan {
    pub(crate) argv: Vec<String>,
    pub(crate) notice: Option<String>,
}

impl LaunchPlan {
    fn plain(args: Vec<String>, reason: &PlainLaunchReason) -> Self {
        Self {
            argv: args,
            notice: reason.notice(),
        }
    }
}

pub(crate) struct LaunchContext {
    pub(crate) working_dir: PathBuf,
    pub(crate) codex_home: Option<PathBuf>,
    pub(crate) sce_executable: Option<PathBuf>,
}

impl LaunchContext {
    fn from_process() -> io::Result<Self> {
        Ok(Self {
            working_dir: std::env::current_dir()?,
            codex_home: std::env::var_os(CODEX_HOME_ENV)
                .map(PathBuf::from)
                .or_else(|| dirs::home_dir().map(|home| home.join(".codex"))),
            sce_executable: std::env::current_exe().ok(),
        })
    }
}

pub(crate) fn classify_launch(args: &[String]) -> LaunchMode {
    let mut tokens = args.iter().map(String::as_str);
    let mut directory = None;
    let mut in_exec = false;
    while let Some(token) = tokens.next() {
        if token == "--" {
            break;
        }
        match read_option(token, &mut tokens) {
            OptionToken::Directory(value) => directory = Some(PathBuf::from(value)),
            OptionToken::Other => {}
            OptionToken::HelpOrVersion => return LaunchMode::Plain(PlainLaunchReason::Sessionless),
            OptionToken::Unrecognized => {
                return LaunchMode::Plain(PlainLaunchReason::UnrecognizedOption(token.to_string()))
            }
            OptionToken::Positional => {
                let subcommand = if in_exec {
                    format!("exec {token}")
                } else {
                    token.to_string()
                };
                match (in_exec, token) {
                    (false, _) if EXEC_SUBCOMMANDS.contains(&token) => in_exec = true,
                    (_, RESUME_SUBCOMMAND) => return classify_resume(directory, tokens),
                    (false, FORK_SUBCOMMAND) => {
                        return LaunchMode::Plain(PlainLaunchReason::InteractiveFork)
                    }
                    (_, "help") => return LaunchMode::Plain(PlainLaunchReason::Sessionless),
                    (false, _) if SESSIONLESS_SUBCOMMANDS.contains(&token) => {
                        return LaunchMode::Plain(PlainLaunchReason::Sessionless)
                    }
                    (_, _) if UNSUPPORTED_SUBCOMMANDS.contains(&token) => {
                        return LaunchMode::Plain(PlainLaunchReason::UnsupportedSubcommand(
                            subcommand,
                        ))
                    }
                    _ => return LaunchMode::Telemetry { directory },
                }
            }
        }
    }
    LaunchMode::Telemetry { directory }
}

fn classify_resume<'a>(
    mut directory: Option<PathBuf>,
    mut tokens: impl Iterator<Item = &'a str>,
) -> LaunchMode {
    while let Some(token) = tokens.next() {
        if let OptionToken::Directory(value) = read_option(token, &mut tokens) {
            directory = Some(PathBuf::from(value));
        }
    }
    match directory {
        Some(directory) => LaunchMode::Telemetry {
            directory: Some(directory),
        },
        None => LaunchMode::Plain(PlainLaunchReason::ResumeWithoutDirectory),
    }
}

enum OptionToken<'a> {
    Directory(&'a str),
    Other,
    HelpOrVersion,
    Unrecognized,
    Positional,
}

fn read_option<'a>(token: &'a str, rest: &mut impl Iterator<Item = &'a str>) -> OptionToken<'a> {
    if token == "-" || !token.starts_with('-') {
        return OptionToken::Positional;
    }
    if HELP_OR_VERSION_OPTIONS.contains(&token) {
        return OptionToken::HelpOrVersion;
    }
    if FLAG_OPTIONS.contains(&token) {
        return OptionToken::Other;
    }
    let Some((name, value)) = split_option(token, rest) else {
        return OptionToken::Unrecognized;
    };
    if DIRECTORY_OPTIONS.contains(&name) {
        value.map_or(OptionToken::Other, OptionToken::Directory)
    } else {
        OptionToken::Other
    }
}

fn split_option<'a>(
    token: &'a str,
    rest: &mut impl Iterator<Item = &'a str>,
) -> Option<(&'a str, Option<&'a str>)> {
    let is_value_option = |name: &str| {
        CONFIG_OPTIONS.contains(&name)
            || PROFILE_OPTIONS.contains(&name)
            || DIRECTORY_OPTIONS.contains(&name)
            || OTHER_VALUE_OPTIONS.contains(&name)
    };
    if is_value_option(token) {
        return Some((token, rest.next()));
    }
    if !token.starts_with("--") && token.len() > 2 && token.is_char_boundary(2) {
        let (name, value) = token.split_at(2);
        let value = value.strip_prefix('=').unwrap_or(value);
        return is_value_option(name).then_some((name, Some(value)));
    }
    let (name, value) = token.split_once('=')?;
    is_value_option(name).then_some((name, Some(value)))
}

fn option_values<'a>(args: &'a [String], names: &[&str]) -> Vec<&'a str> {
    let mut values = Vec::new();
    let mut tokens = args.iter().map(String::as_str);
    while let Some(token) = tokens.next() {
        if token == "--" {
            break;
        }
        if let Some((name, Some(value))) = split_option(token, &mut tokens) {
            if names.contains(&name) {
                values.push(value);
            }
        }
    }
    values
}

pub(crate) fn detect_otel_conflict<R, E>(
    args: &[String],
    codex_home: Option<&Path>,
    read_config: R,
    environment: E,
) -> Option<OtelConflict>
where
    R: Fn(&Path) -> io::Result<Option<String>>,
    E: Fn(&str) -> Option<String>,
{
    if visible_destination_conflict(environment) {
        return Some(OtelConflict::DestinationEnvironment);
    }
    if option_values(args, &CONFIG_OPTIONS)
        .iter()
        .any(|value| is_otel_key(value.split_once('=').map_or(*value, |(key, _)| key)))
    {
        return Some(OtelConflict::CallerOverride);
    }
    let Some(codex_home) = codex_home else {
        return Some(OtelConflict::UnknownCodexHome);
    };

    let user_path = codex_home.join("config.toml");
    let user_config = match load_toml(&user_path, &read_config) {
        Ok(config) => config.unwrap_or_default(),
        Err(()) => return Some(OtelConflict::UnreadableConfig(user_path)),
    };
    if defines_user_owned_otel(&user_config) {
        return Some(OtelConflict::UserConfig(user_path));
    }

    let profile = option_values(args, &PROFILE_OPTIONS)
        .last()
        .map(|name| (*name).to_string())
        .or_else(|| {
            user_config
                .get("profile")
                .and_then(toml::Value::as_str)
                .map(str::to_string)
        });
    if let Some(profile) = profile {
        let inline_profile = user_config
            .get("profiles")
            .and_then(|profiles| profiles.get(&profile))
            .and_then(toml::Value::as_table);
        if inline_profile.is_some_and(defines_user_owned_otel) {
            return Some(OtelConflict::ProfileConfig(user_path));
        }
        let profile_path = codex_home.join(format!("{profile}.config.toml"));
        match load_toml(&profile_path, &read_config) {
            Ok(Some(config)) if defines_user_owned_otel(&config) => {
                return Some(OtelConflict::ProfileConfig(profile_path))
            }
            Ok(_) => {}
            Err(()) => return Some(OtelConflict::UnreadableConfig(profile_path)),
        }
    }

    SYSTEM_CONFIG_PATHS
        .iter()
        .map(Path::new)
        .find_map(|path| match read_config(path) {
            Ok(Some(contents)) => match contents.parse::<toml::Table>() {
                Ok(config) if !config.contains_key("otel") => None,
                _ => Some(OtelConflict::SystemConfig(path.to_path_buf())),
            },
            Ok(None) | Err(_) => None,
        })
}

fn is_otel_key(key: &str) -> bool {
    let key = key.trim();
    key == "otel" || key.starts_with("otel.")
}

fn load_toml<R>(path: &Path, read_config: &R) -> Result<Option<toml::Table>, ()>
where
    R: Fn(&Path) -> io::Result<Option<String>>,
{
    match read_config(path) {
        Ok(Some(contents)) => contents.parse::<toml::Table>().map(Some).map_err(|_| ()),
        Ok(None) => Ok(None),
        Err(_) => Err(()),
    }
}

fn defines_user_owned_otel(config: &toml::Table) -> bool {
    config
        .get("otel")
        .and_then(toml::Value::as_table)
        .is_some_and(|otel| {
            USER_OWNED_OTEL_KEYS
                .iter()
                .any(|key| otel.contains_key(*key))
        })
}

pub(crate) fn read_config_file(path: &Path) -> io::Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(contents) => Ok(Some(contents)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

pub(crate) fn telemetry_overrides(sce_executable: &Path, repository_root: &Path) -> Vec<String> {
    let helper = format!(
        "{} auth telemetry-headers --repo {}",
        shell_quote(&sce_executable.to_string_lossy()),
        shell_quote(&repository_root.to_string_lossy()),
    );
    let exporter = |endpoint: &str| {
        format!(
            "{{otlp-http={{endpoint={},protocol={}}}}}",
            toml_string(endpoint),
            toml_string(OTLP_HTTP_BINARY_PROTOCOL)
        )
    };
    [
        ("otel.headers_helper", toml_string(&helper)),
        (
            "otel.headers_helper_refresh_interval_ms",
            HEADERS_HELPER_REFRESH_INTERVAL_MS.to_string(),
        ),
        ("otel.exporter", toml_string("none")),
        (
            "otel.trace_exporter",
            exporter(MANAGED_OTLP_TRACES_ENDPOINT),
        ),
        (
            "otel.metrics_exporter",
            exporter(MANAGED_OTLP_METRICS_ENDPOINT),
        ),
        ("otel.log_user_prompt", "false".to_string()),
        ("otel.log_agent_responses", "false".to_string()),
        ("otel.log_guardian_assessments", "false".to_string()),
    ]
    .into_iter()
    .flat_map(|(key, value)| ["-c".to_string(), format!("{key}={value}")])
    .collect()
}

fn toml_string(value: &str) -> String {
    toml::Value::String(value.to_string()).to_string()
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

pub(crate) fn repository_root_for(launch_dir: &Path) -> Option<PathBuf> {
    let launch_dir = std::fs::canonicalize(launch_dir).ok()?;
    enclosing_repository_root(&launch_dir).map(Path::to_path_buf)
}

pub(crate) async fn plan_launch_with<R, E, G, P, A, AF>(
    args: Vec<String>,
    context: &LaunchContext,
    read_config: R,
    environment: E,
    resolve_repository_root: G,
    telemetry_enabled: P,
    authorized: A,
) -> LaunchPlan
where
    R: Fn(&Path) -> io::Result<Option<String>>,
    E: Fn(&str) -> Option<String>,
    G: FnOnce(&Path) -> Option<PathBuf>,
    P: FnOnce(&Path) -> bool,
    A: FnOnce(PathBuf) -> AF,
    AF: Future<Output = bool>,
{
    let directory = match classify_launch(&args) {
        LaunchMode::Plain(reason) => return LaunchPlan::plain(args, &reason),
        LaunchMode::Telemetry { directory } => directory,
    };
    let launch_dir = directory.map_or_else(
        || context.working_dir.clone(),
        |directory| context.working_dir.join(directory),
    );
    let Some(repository_root) = resolve_repository_root(&launch_dir) else {
        return LaunchPlan::plain(args, &PlainLaunchReason::NotRepository);
    };
    if !telemetry_enabled(&repository_root) {
        return LaunchPlan::plain(args, &PlainLaunchReason::TelemetryDisabled);
    }
    if let Some(conflict) = detect_otel_conflict(
        &args,
        context.codex_home.as_deref(),
        read_config,
        environment,
    ) {
        return LaunchPlan::plain(args, &PlainLaunchReason::Conflict(conflict));
    }
    let Some(sce_executable) = context.sce_executable.as_deref() else {
        return LaunchPlan::plain(args, &PlainLaunchReason::ExecutableUnavailable);
    };
    let overrides = telemetry_overrides(sce_executable, &repository_root);
    if !authorized(repository_root).await {
        return LaunchPlan::plain(args, &PlainLaunchReason::CredentialUnavailable);
    }
    let mut launch_argv = overrides;
    launch_argv.extend(args);
    LaunchPlan {
        argv: launch_argv,
        notice: None,
    }
}

#[cfg(test)]
mod tests;
