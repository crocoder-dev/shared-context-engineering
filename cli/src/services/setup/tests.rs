use super::*;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::command_surface;
use crate::services::command_registry::CommandRegistry;
use crate::services::command_registry::RuntimeCommand;
use crate::services::parse::command_runtime::parse_runtime_command;

fn options_with(mutate: impl FnOnce(&mut SetupCliOptions)) -> SetupCliOptions {
    let mut options = SetupCliOptions::default();
    mutate(&mut options);
    options
}

fn unique_temp_dir(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after Unix epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "sce-setup-context-{label}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn init_git_repo(label: &str) -> PathBuf {
    let repo = unique_temp_dir(label);
    let output = Command::new("git")
        .args(["init", "-q"])
        .current_dir(&repo)
        .output()
        .expect("git init should spawn");
    assert!(
        output.status.success(),
        "git init failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    repo
}

fn assert_baseline_paths_exist(repo: &Path) {
    let paths = RepoPaths::new(repo);
    for path in [
        paths.context_overview_file(),
        paths.context_architecture_file(),
        paths.context_patterns_file(),
        paths.context_glossary_file(),
        paths.context_map_file(),
        paths.context_plans_dir(),
        paths.context_handovers_dir(),
        paths.context_decisions_dir(),
        paths.context_tmp_dir(),
        paths.context_tmp_gitignore_file(),
    ] {
        assert!(path.exists(), "expected baseline path {}", path.display());
    }
}

#[test]
fn repo_local_config_bootstrap_payload_uses_versioned_schema_url() {
    let payload = repo_local_config_bootstrap_payload();

    assert_eq!(
            payload,
            format!(
                "{{\n  \"$schema\": \"https://sce.crocoder.dev/v{}/config.json\",\n  \"agent_trace\": {{\n    \"auto_sync\": true\n  }},\n  \"policies\": {{\n    \"attribution_hooks\": {{\n      \"enabled\": true\n    }}\n  }}\n}}\n",
                env!("CARGO_PKG_VERSION")
            )
        );
}

#[test]
fn resolve_setup_request_accepts_pi_target() {
    let request = resolve_setup_request(options_with(|options| {
        options.pi = true;
        options.non_interactive = true;
    }))
    .expect("pi target should resolve");

    assert_eq!(
        request.config_mode,
        Some(SetupMode::NonInteractive(SetupTarget::Pi))
    );
    assert!(!request.context_only);
}

#[test]
fn resolve_setup_request_accepts_codex_target() {
    let request = resolve_setup_request(options_with(|options| {
        options.codex = true;
        options.non_interactive = true;
    }))
    .expect("codex target should resolve");

    assert_eq!(
        request.config_mode,
        Some(SetupMode::NonInteractive(SetupTarget::Codex))
    );
    assert!(!request.context_only);
}

#[test]
fn resolve_setup_request_accepts_all_target() {
    let request = resolve_setup_request(options_with(|options| {
        options.all = true;
        options.non_interactive = true;
    }))
    .expect("all target should resolve");

    assert_eq!(
        request.config_mode,
        Some(SetupMode::NonInteractive(SetupTarget::All))
    );
    assert!(!request.context_only);
}

#[test]
fn resolve_setup_request_accepts_bootstrap_context_alone() {
    let request = resolve_setup_request(options_with(|options| {
        options.bootstrap_context = true;
    }))
    .expect("bootstrap-context alone should resolve");

    assert!(request.context_only);
    assert_eq!(request.config_mode, None);
    assert!(!request.install_hooks);
    assert_eq!(request.hooks_repo_path, None);
}

#[test]
fn resolve_setup_request_rejects_bootstrap_context_with_target() {
    let error = resolve_setup_request(options_with(|options| {
        options.bootstrap_context = true;
        options.opencode = true;
    }))
    .expect_err("bootstrap-context with target must be rejected");

    assert!(error.to_string().contains("--bootstrap-context"));
    assert!(error.to_string().contains("alone"));
}

#[test]
fn resolve_setup_request_rejects_combined_target_flags() {
    let error = resolve_setup_request(options_with(|options| {
        options.pi = true;
        options.all = true;
    }))
    .expect_err("combined target flags must be rejected");

    assert!(error.to_string().contains("mutually exclusive"));
}

#[test]
fn resolve_setup_request_non_interactive_error_lists_pi_and_all() {
    let error = resolve_setup_request(options_with(|options| {
        options.non_interactive = true;
    }))
    .expect_err("non-interactive without target must be rejected");

    let message = error.to_string();
    assert!(message.contains("--pi"));
    assert!(message.contains("--all"));
}

#[test]
fn parser_routes_bootstrap_context_to_context_only_request() {
    let registry = CommandRegistry::default();
    let command = parse_runtime_command(
        [
            "sce".to_string(),
            "setup".to_string(),
            "--bootstrap-context".to_string(),
        ],
        &registry,
        None,
    )
    .expect("bootstrap-context should parse");

    match command {
        RuntimeCommand::Setup(setup_command) => {
            assert!(setup_command.request.context_only);
            assert_eq!(setup_command.request.config_mode, None);
            assert!(!setup_command.request.install_hooks);
        }
        _ => panic!("expected Setup command for --bootstrap-context"),
    }
}

#[test]
fn help_documents_bootstrap_context_flag() {
    let top_level_help = command_surface::help_text();
    assert!(
        top_level_help.contains("--bootstrap-context"),
        "top-level help should document --bootstrap-context"
    );

    let registry = CommandRegistry::default();
    let command = parse_runtime_command(
        ["sce".to_string(), "setup".to_string(), "--help".to_string()],
        &registry,
        None,
    )
    .expect("setup --help should parse");

    match command {
        RuntimeCommand::HelpText(help) => {
            assert!(
                help.text.contains("--bootstrap-context"),
                "setup --help should document --bootstrap-context:\n{}",
                help.text
            );
        }
        _ => panic!("expected HelpText for setup --help"),
    }
}

#[test]
fn bootstrap_context_baseline_creates_expected_paths() {
    let repo = init_git_repo("create-baseline");
    let message = bootstrap_context_baseline(&repo).expect("bootstrap should create baseline");
    assert!(message.contains("Context baseline ensured."));
    assert_baseline_paths_exist(&repo);

    let paths = RepoPaths::new(&repo);
    assert!(!paths.opencode_dir().exists());
    assert!(!paths.claude_dir().exists());
    assert!(!paths.pi_dir().exists());

    let gitignore = fs::read_to_string(paths.context_tmp_gitignore_file())
        .expect("tmp gitignore should be readable");
    assert_eq!(gitignore, CONTEXT_TMP_GITIGNORE_CONTENT);

    let _ = fs::remove_dir_all(&repo);
}

#[test]
fn bootstrap_context_baseline_is_additive_and_idempotent() {
    let repo = init_git_repo("idempotent-baseline");
    bootstrap_context_baseline(&repo).expect("initial bootstrap");

    let paths = RepoPaths::new(&repo);
    let sentinel = "SENTINEL_OVERVIEW_CONTENT\n";
    fs::write(paths.context_overview_file(), sentinel).expect("seed overview sentinel");
    fs::write(paths.context_map_file(), "SENTINEL_CONTEXT_MAP\n")
        .expect("seed context-map sentinel");
    fs::write(paths.context_tmp_gitignore_file(), "SENTINEL_GITIGNORE\n")
        .expect("seed gitignore sentinel");

    fs::remove_file(paths.context_architecture_file()).expect("remove architecture");
    fs::remove_dir_all(paths.context_plans_dir()).expect("remove plans");

    bootstrap_context_baseline(&repo).expect("rerun bootstrap");

    assert_eq!(
        fs::read_to_string(paths.context_overview_file()).expect("read overview"),
        sentinel
    );
    assert_eq!(
        fs::read_to_string(paths.context_map_file()).expect("read context-map"),
        "SENTINEL_CONTEXT_MAP\n"
    );
    assert_eq!(
        fs::read_to_string(paths.context_tmp_gitignore_file()).expect("read gitignore"),
        "SENTINEL_GITIGNORE\n"
    );
    assert!(paths.context_architecture_file().exists());
    assert!(paths.context_plans_dir().is_dir());

    let _ = fs::remove_dir_all(&repo);
}

#[test]
fn concrete_targets_for_all_expands_to_four_targets() {
    assert_eq!(
        concrete_targets_for(SetupTarget::All),
        &[
            SetupTarget::OpenCode,
            SetupTarget::Claude,
            SetupTarget::Pi,
            SetupTarget::Codex
        ]
    );
}

#[test]
fn integration_target_id_str_maps_pi() {
    assert_eq!(integration_target_id_str(SetupTarget::Pi), "pi");
}

#[test]
fn integration_target_id_str_maps_codex() {
    assert_eq!(integration_target_id_str(SetupTarget::Codex), "codex");
}

fn every_optional_workflow() -> Vec<&'static str> {
    super::OPTIONAL_WORKFLOWS
        .iter()
        .map(|workflow| workflow.id)
        .collect()
}

#[test]
fn iter_embedded_assets_for_all_covers_each_concrete_target() {
    let selection = every_optional_workflow();
    let count =
        |target| iter_embedded_assets_for_setup_target_with_selection(target, &selection).count();

    let concrete_sum = count(SetupTarget::OpenCode)
        + count(SetupTarget::Claude)
        + count(SetupTarget::Pi)
        + count(SetupTarget::Codex);

    assert!(count(SetupTarget::Pi) > 0);
    assert!(count(SetupTarget::Codex) > 0);
    assert_eq!(count(SetupTarget::All), concrete_sum);
}

#[test]
fn embedded_build_payload_contains_generated_targets_and_static_hooks() {
    let selection = every_optional_workflow();
    let contains = |target, path| {
        iter_embedded_assets_for_setup_target_with_selection(target, &selection)
            .any(|asset| asset.relative_path == path && !asset.bytes.is_empty())
    };

    assert!(contains(SetupTarget::OpenCode, "command/next-task.md"));
    assert!(contains(
        SetupTarget::OpenCode,
        "lib/bash-policy-presets.json"
    ));
    assert!(contains(SetupTarget::Claude, "commands/next-task.md"));
    assert!(contains(SetupTarget::Pi, "prompts/next-task.md"));
    assert!(contains(SetupTarget::Pi, "extensions/sce/index.ts"));
    assert!(iter_required_hook_assets().all(|asset| !asset.bytes.is_empty()));
}

#[test]
fn codex_embedded_assets_cover_both_output_roots_with_no_command_dir() {
    let has = |path: &str| {
        CODEX_EMBEDDED_ASSETS
            .iter()
            .any(|asset| asset.relative_path == path && !asset.bytes.is_empty())
    };

    assert!(has(".agents/skills/sce-next-task/SKILL.md"));
    assert!(has(".codex/hooks.json"));
    assert!(has(".codex/hooks/run-sce-or-show-install-guidance.sh"));
    assert!(!CODEX_EMBEDDED_ASSETS
        .iter()
        .any(|asset| asset.relative_path.starts_with(".agents/commands/")));
}

#[test]
fn install_writes_codex_assets_directly_under_repo_root() {
    let repo = init_git_repo("install-codex-dual-roots");
    let selection: Vec<String> = every_optional_workflow()
        .into_iter()
        .map(str::to_string)
        .collect();

    install_embedded_setup_assets(&repo, SetupTarget::Codex, &selection)
        .expect("codex install should succeed");

    assert!(repo.join(".agents/skills/sce-next-task/SKILL.md").is_file());
    assert!(repo.join(".codex/hooks.json").is_file());
    assert!(repo
        .join(".codex/hooks/run-sce-or-show-install-guidance.sh")
        .is_file());
    assert!(!repo.join(".codex/.agents").exists());
    assert!(!repo.join(".agents/.codex").exists());

    let _ = fs::remove_dir_all(&repo);
}

#[test]
fn install_merges_codex_hooks_and_replaces_stale_owned_handlers_idempotently() {
    let repo = init_git_repo("install-merges-codex-hooks");
    let hooks_path = repo.join(".codex/hooks.json");
    fs::create_dir_all(hooks_path.parent().unwrap()).expect("create Codex directory");
    let stale_command = "bash .codex/hooks/run-sce-or-show-install-guidance.sh sce hooks codex";
    let existing = json!({
        "description": "user hooks",
        "hooks": {
            "UserPromptSubmit": [{"hooks": [
                {"type": "command", "command": "echo user"},
                {"type": "command", "command": stale_command}
            ]}],
            "SessionStart": [{"hooks": [{"type": "command", "command": "echo session"}]}]
        }
    });
    fs::write(&hooks_path, serde_json::to_vec(&existing).unwrap()).expect("seed hooks config");
    let selection: Vec<String> = every_optional_workflow()
        .into_iter()
        .map(str::to_string)
        .collect();

    install_embedded_setup_assets(&repo, SetupTarget::Codex, &selection)
        .expect("first Codex install should succeed");
    let first = fs::read(&hooks_path).expect("read merged hooks config");
    install_embedded_setup_assets(&repo, SetupTarget::Codex, &selection)
        .expect("second Codex install should succeed");
    let second = fs::read(&hooks_path).expect("read merged hooks config again");
    assert_eq!(first, second);

    let merged: serde_json::Value = serde_json::from_slice(&second).unwrap();
    assert_eq!(merged["description"], "user hooks");
    assert_eq!(
        merged["hooks"]["SessionStart"][0]["hooks"][0]["command"],
        "echo session"
    );
    assert_eq!(
        merged["hooks"]["UserPromptSubmit"][0]["hooks"][0]["command"],
        "echo user"
    );
    assert_eq!(merged["hooks"].as_object().unwrap().len(), 8);
    for event in ["Interrupt", "SubagentStop", "SessionEnd"] {
        assert!(
            merged["hooks"][event][0]["hooks"][0]["command"]
                .as_str()
                .unwrap()
                .ends_with("sce hooks codex-mutation-scope"),
            "{event} must route to the mutation-scope command"
        );
    }
    assert!(merged["hooks"]["PreToolUse"][1]["hooks"][0]["command"]
        .as_str()
        .unwrap()
        .ends_with("sce hooks codex-mutation-scope"));

    let _ = fs::remove_dir_all(&repo);
}

#[test]
fn invalid_codex_hooks_are_not_modified() {
    let invalid_documents = [
        br#"{\"hooks\":{"#.to_vec(),
        serde_json::to_vec(&json!({"custom": true})).unwrap(),
        serde_json::to_vec(&json!({"hooks": {"Stop": [{"matcher": 42}]}})).unwrap(),
        serde_json::to_vec(&json!({"hooks": {"Stop": [{"hooks": "invalid"}]}})).unwrap(),
        serde_json::to_vec(&json!({"hooks": {"Stop": [{"hooks": [{"nonsense": true}]}]}})).unwrap(),
        serde_json::to_vec(&json!({"hooks": {"Stop": [{"hooks": [{"type": "unknown"}]}]}}))
            .unwrap(),
    ];
    let selection: Vec<String> = every_optional_workflow()
        .into_iter()
        .map(str::to_string)
        .collect();

    for (index, original) in invalid_documents.iter().enumerate() {
        let repo = init_git_repo(&format!("install-rejects-malformed-codex-hooks-{index}"));
        let hooks_path = repo.join(".codex/hooks.json");
        fs::create_dir_all(hooks_path.parent().unwrap()).expect("create Codex directory");
        fs::write(&hooks_path, original).expect("seed malformed hooks config");

        let error = install_embedded_setup_assets(&repo, SetupTarget::Codex, &selection)
            .expect_err("malformed Codex hooks should fail setup");
        assert!(error.to_string().contains(".codex/hooks.json"));
        assert_eq!(fs::read(&hooks_path).unwrap(), original.as_slice());

        let _ = fs::remove_dir_all(&repo);
    }
}

#[test]
fn install_preserves_user_owned_files_and_writes_sce_assets() {
    let repo = init_git_repo("install-preserves-user-files");
    let claude_dir = default_paths::InstallTargetPaths::new(&repo).claude_target_dir();

    fs::create_dir_all(claude_dir.join("skills/my-own-skill")).expect("create user skill dir");
    fs::create_dir_all(claude_dir.join("commands")).expect("create commands dir");

    fs::write(claude_dir.join("MY_NOTES.md"), "top level user notes\n")
        .expect("seed top-level user file");
    fs::write(
        claude_dir.join("skills/my-own-skill/SKILL.md"),
        "user skill content\n",
    )
    .expect("seed user skill file");
    fs::write(
        claude_dir.join("commands/my-command.md"),
        "user command content\n",
    )
    .expect("seed user command file");

    let selection: Vec<String> = every_optional_workflow()
        .into_iter()
        .map(str::to_string)
        .collect();

    install_embedded_setup_assets(&repo, SetupTarget::Claude, &selection)
        .expect("install should succeed");

    assert_eq!(
        fs::read_to_string(claude_dir.join("MY_NOTES.md")).expect("read top-level user file"),
        "top level user notes\n"
    );
    assert_eq!(
        fs::read_to_string(claude_dir.join("skills/my-own-skill/SKILL.md"))
            .expect("read user skill file"),
        "user skill content\n"
    );
    assert_eq!(
        fs::read_to_string(claude_dir.join("commands/my-command.md"))
            .expect("read user command file"),
        "user command content\n"
    );

    let expected_next_task_bytes =
        iter_embedded_assets_for_setup_target_with_selection(SetupTarget::Claude, &selection)
            .find(|asset| asset.relative_path == "commands/next-task.md")
            .expect("next-task asset should be in the catalog")
            .bytes;
    assert_eq!(
        fs::read(claude_dir.join("commands/next-task.md")).expect("read installed sce asset"),
        expected_next_task_bytes
    );

    let _ = fs::remove_dir_all(&repo);
}

#[test]
fn install_merges_into_existing_claude_settings_json_and_stays_idempotent() {
    let repo = init_git_repo("install-merges-claude-settings");
    let claude_dir = default_paths::InstallTargetPaths::new(&repo).claude_target_dir();

    fs::create_dir_all(&claude_dir).expect("create claude dir");
    fs::write(
        claude_dir.join("settings.json"),
        serde_json::to_string_pretty(&json!({
            "permissions": {"allow": ["Bash(git *)"]},
            "env": {"FOO": "bar"},
            "hooks": {
                "PreToolUse": [
                    {
                        "matcher": "Bash",
                        "hooks": [{"type": "command", "command": "echo user-hook"}]
                    }
                ]
            }
        }))
        .expect("serialize seeded settings"),
    )
    .expect("seed existing settings.json");

    let selection: Vec<String> = every_optional_workflow()
        .into_iter()
        .map(str::to_string)
        .collect();

    install_embedded_setup_assets(&repo, SetupTarget::Claude, &selection)
        .expect("first install should succeed");

    let after_first =
        fs::read_to_string(claude_dir.join("settings.json")).expect("read merged settings");
    let merged: serde_json::Value =
        serde_json::from_str(&after_first).expect("merged settings should be valid JSON");

    assert_eq!(merged["permissions"]["allow"][0], "Bash(git *)");
    assert_eq!(merged["env"]["FOO"], "bar");
    let pre_tool_use = merged["hooks"]["PreToolUse"]
        .as_array()
        .expect("PreToolUse should be an array");
    assert!(pre_tool_use
        .iter()
        .any(|entry| entry["hooks"][0]["command"] == "echo user-hook"));
    assert!(pre_tool_use
        .iter()
        .any(|entry| entry["hooks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|hook| hook["command"]
                .as_str()
                .unwrap()
                .contains("run-sce-or-show-install-guidance.sh"))));

    install_embedded_setup_assets(&repo, SetupTarget::Claude, &selection)
        .expect("second install should succeed");

    let after_second =
        fs::read_to_string(claude_dir.join("settings.json")).expect("read re-merged settings");
    assert_eq!(
        after_first, after_second,
        "two consecutive installs should merge to byte-identical output"
    );

    let _ = fs::remove_dir_all(&repo);
}

#[test]
fn install_merges_into_existing_opencode_config_json_and_stays_idempotent() {
    let repo = init_git_repo("install-merges-opencode-config");
    let opencode_dir = default_paths::InstallTargetPaths::new(&repo).opencode_target_dir();

    fs::create_dir_all(&opencode_dir).expect("create opencode dir");
    fs::write(
        opencode_dir.join("opencode.json"),
        serde_json::to_string_pretty(&json!({
            "model": "anthropic/claude",
            "mcp": {"my-server": {"command": "my-server"}},
            "plugin": ["./plugins/my-plugin.ts", "./plugins/sce-old-feature.ts"]
        }))
        .expect("serialize seeded opencode config"),
    )
    .expect("seed existing opencode.json");

    let selection: Vec<String> = every_optional_workflow()
        .into_iter()
        .map(str::to_string)
        .collect();

    install_embedded_setup_assets(&repo, SetupTarget::OpenCode, &selection)
        .expect("first install should succeed");

    let after_first = fs::read_to_string(opencode_dir.join("opencode.json"))
        .expect("read merged opencode config");
    let merged: serde_json::Value =
        serde_json::from_str(&after_first).expect("merged opencode config should be valid JSON");

    assert_eq!(merged["model"], "anthropic/claude");
    assert_eq!(merged["mcp"]["my-server"]["command"], "my-server");

    let plugin = merged["plugin"]
        .as_array()
        .expect("plugin should be an array");
    assert!(plugin.contains(&json!("./plugins/my-plugin.ts")));
    assert!(plugin.contains(&json!("./plugins/sce-bash-policy.ts")));
    assert!(plugin.contains(&json!("./plugins/sce-agent-trace.ts")));
    assert!(!plugin.contains(&json!("./plugins/sce-old-feature.ts")));
    assert_eq!(
        plugin.last().and_then(serde_json::Value::as_str),
        Some("./plugins/sce-mutation-scope.ts"),
        "the mutation-scope plugin must be installed as the final plugin"
    );

    install_embedded_setup_assets(&repo, SetupTarget::OpenCode, &selection)
        .expect("second install should succeed");

    let after_second = fs::read_to_string(opencode_dir.join("opencode.json"))
        .expect("read re-merged opencode config");
    assert_eq!(
        after_first, after_second,
        "two consecutive installs should merge to byte-identical output"
    );

    let _ = fs::remove_dir_all(&repo);
}

#[test]
fn reinstall_with_empty_selection_prunes_deselected_workflow_without_touching_sibling_skill() {
    let repo = init_git_repo("install-prunes-deselected-workflow");
    let claude_dir = default_paths::InstallTargetPaths::new(&repo).claude_target_dir();

    let brownfield_selection = vec!["brownfield".to_string()];
    install_embedded_setup_assets(&repo, SetupTarget::Claude, &brownfield_selection)
        .expect("initial install with brownfield selected should succeed");

    let brownfield_command = claude_dir.join("commands/brownfield.md");
    let brownfield_skill_dir = claude_dir.join("skills/sce-brownfield");
    assert!(
        brownfield_command.is_file(),
        "brownfield command should be installed"
    );
    assert!(
        brownfield_skill_dir.is_dir(),
        "brownfield skill dir should be installed"
    );

    fs::create_dir_all(claude_dir.join("skills/my-skill")).expect("create user skill dir");
    fs::write(
        claude_dir.join("skills/my-skill/SKILL.md"),
        "sibling user skill\n",
    )
    .expect("seed sibling user skill file");

    install_embedded_setup_assets(&repo, SetupTarget::Claude, &[])
        .expect("reinstall with empty selection should succeed");

    assert!(
        !brownfield_command.exists(),
        "deselected workflow command should be pruned"
    );
    assert!(
        !brownfield_skill_dir.exists(),
        "deselected workflow skill dir should be pruned entirely once empty"
    );
    assert_eq!(
        fs::read_to_string(claude_dir.join("skills/my-skill/SKILL.md"))
            .expect("read sibling user skill file"),
        "sibling user skill\n"
    );

    let _ = fs::remove_dir_all(&repo);
}

#[test]
fn reinstall_with_empty_selection_keeps_pruned_skill_dir_holding_a_user_file() {
    let repo = init_git_repo("install-prunes-but-keeps-user-file");
    let claude_dir = default_paths::InstallTargetPaths::new(&repo).claude_target_dir();

    let brownfield_selection = vec!["brownfield".to_string()];
    install_embedded_setup_assets(&repo, SetupTarget::Claude, &brownfield_selection)
        .expect("initial install with brownfield selected should succeed");

    let brownfield_skill_dir = claude_dir.join("skills/sce-brownfield");
    fs::write(
        brownfield_skill_dir.join("MY_OVERRIDE.md"),
        "user file inside sce skill dir\n",
    )
    .expect("seed user file inside sce-owned skill dir");

    install_embedded_setup_assets(&repo, SetupTarget::Claude, &[])
        .expect("reinstall with empty selection should succeed");

    assert!(
        !brownfield_skill_dir.join("SKILL.md").exists(),
        "deselected workflow skill file should be pruned"
    );
    assert!(
        brownfield_skill_dir.is_dir(),
        "sce-owned skill dir should survive because it still holds a user file"
    );
    assert_eq!(
        fs::read_to_string(brownfield_skill_dir.join("MY_OVERRIDE.md"))
            .expect("read user file inside pruned skill dir"),
        "user file inside sce skill dir\n"
    );

    let _ = fs::remove_dir_all(&repo);
}

#[test]
fn install_cleans_up_staging_and_reports_asset_path_on_rename_failure() {
    let repo = init_git_repo("install-rename-failure");
    let selection: Vec<String> = every_optional_workflow()
        .into_iter()
        .map(str::to_string)
        .collect();

    let claude_dir = default_paths::InstallTargetPaths::new(&repo).claude_target_dir();
    let failing_destination = claude_dir.join("commands/next-task.md");

    fs::create_dir_all(claude_dir.join("commands")).expect("create commands dir");
    let prior_content = b"prior next-task content\n";
    fs::write(&failing_destination, prior_content).expect("seed prior next-task content");

    let result = install::install_embedded_setup_assets_with_rename(
        &repo,
        SetupTarget::Claude,
        &selection,
        |from, to| {
            if to == failing_destination {
                Err(std::io::Error::other("simulated rename failure"))
            } else {
                fs::rename(from, to)
            }
        },
    );

    let error = result.expect_err("rename failure should surface as an error");
    let message = format!("{error:#}");
    assert!(
        message.contains(&failing_destination.display().to_string()),
        "error should name the failing asset path: {message}"
    );
    assert!(
        message.contains("does not create backups"),
        "error should include recovery guidance: {message}"
    );

    assert_eq!(
        fs::read(&failing_destination).expect("read failing destination after rename failure"),
        prior_content,
        "prior content at the failing destination should survive a rename failure"
    );

    let commands_staging_dir = claude_dir.join("commands");
    if commands_staging_dir.exists() {
        let leftover_staging_files = fs::read_dir(&commands_staging_dir)
            .expect("read commands staging dir")
            .filter_map(Result::ok)
            .any(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".sce-setup-staging-")
            });
        assert!(
            !leftover_staging_files,
            "staging artifact for the failed asset should be cleaned up"
        );
    }

    let _ = fs::remove_dir_all(&repo);
}

#[test]
fn hook_install_leaves_prior_hook_intact_on_rename_failure() {
    let repo = init_git_repo("hook-install-rename-failure");

    let initial_outcome =
        install::install_required_git_hooks(&repo).expect("initial hook install should succeed");
    let pre_commit_result = initial_outcome
        .hook_results
        .iter()
        .find(|result| result.hook_name == default_paths::hook_dir::PRE_COMMIT)
        .expect("pre-commit hook should be installed");
    let pre_commit_path = pre_commit_result.hook_path.clone();

    let prior_hook_bytes = b"#!/bin/sh\necho prior pre-commit\n".to_vec();
    fs::write(&pre_commit_path, &prior_hook_bytes).expect("seed prior pre-commit hook");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&pre_commit_path, fs::Permissions::from_mode(0o755))
            .expect("mark prior pre-commit hook executable");
    }
    let prior_mode = fs::metadata(&pre_commit_path)
        .expect("stat prior pre-commit hook")
        .permissions();

    let result = install::install_required_git_hooks_with_rename(&repo, |from, to| {
        if to == pre_commit_path {
            Err(std::io::Error::other("simulated rename failure"))
        } else {
            fs::rename(from, to)
        }
    });

    let error = result.expect_err("rename failure should surface as an error");
    let message = format!("{error:#}");
    assert!(
        message.contains(&pre_commit_path.display().to_string()),
        "error should name the failing hook path: {message}"
    );

    assert_eq!(
        fs::read(&pre_commit_path).expect("read pre-commit hook after rename failure"),
        prior_hook_bytes,
        "prior hook content should survive a rename failure"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode_after = fs::metadata(&pre_commit_path)
            .expect("stat pre-commit hook after rename failure")
            .permissions();
        assert_eq!(
            mode_after.mode() & 0o777,
            prior_mode.mode() & 0o777,
            "prior hook executable mode should survive a rename failure"
        );
    }

    let hooks_staging_dir = pre_commit_path
        .parent()
        .expect("pre-commit hook should have a parent directory");
    let leftover_staging_files = fs::read_dir(hooks_staging_dir)
        .expect("read hooks staging dir")
        .filter_map(Result::ok)
        .any(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(".sce-hook-staging-")
        });
    assert!(
        !leftover_staging_files,
        "staging artifact for the failed hook should be cleaned up"
    );

    let _ = fs::remove_dir_all(&repo);
}

#[test]
fn foreign_pre_commit_hook_keeps_its_content_and_gains_the_sce_block() {
    let repo = init_git_repo("hook-install-foreign-append");

    let initial_outcome =
        install::install_required_git_hooks(&repo).expect("initial hook install should succeed");
    let pre_commit_path = initial_outcome
        .hook_results
        .iter()
        .find(|result| result.hook_name == default_paths::hook_dir::PRE_COMMIT)
        .expect("pre-commit hook should be installed")
        .hook_path
        .clone();

    let foreign_bytes = b"#!/bin/sh\necho husky-style-guard\n".to_vec();
    fs::write(&pre_commit_path, &foreign_bytes).expect("seed foreign pre-commit hook");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&pre_commit_path, fs::Permissions::from_mode(0o755))
            .expect("mark foreign pre-commit hook executable");
    }

    let outcome = install::install_required_git_hooks(&repo)
        .expect("hook install over a foreign hook should succeed");
    let result = outcome
        .hook_results
        .iter()
        .find(|result| result.hook_name == default_paths::hook_dir::PRE_COMMIT)
        .expect("pre-commit hook result should be present");

    assert_eq!(result.status, RequiredHookInstallStatus::Updated);
    assert!(!result.unreachable_block_advisory);

    let installed_bytes = fs::read(&pre_commit_path).expect("read installed pre-commit hook");
    assert!(
        installed_bytes.starts_with(&foreign_bytes),
        "foreign hook content should survive as an exact prefix"
    );
    let installed_text = String::from_utf8(installed_bytes).expect("hook should be utf8");
    assert!(installed_text.contains(hook_merge::MANAGED_BLOCK_START));
    assert!(installed_text.contains(hook_merge::MANAGED_BLOCK_END));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&pre_commit_path)
            .expect("stat installed pre-commit hook")
            .permissions()
            .mode();
        assert_ne!(mode & 0o111, 0, "installed hook should remain executable");
    }

    let _ = fs::remove_dir_all(&repo);
}

#[test]
fn rerunning_hook_install_is_idempotent_for_block_only_and_foreign_plus_block_shapes() {
    let repo = init_git_repo("hook-install-idempotent");

    let first_outcome =
        install::install_required_git_hooks(&repo).expect("first hook install should succeed");
    let pre_commit_result = first_outcome
        .hook_results
        .iter()
        .find(|result| result.hook_name == default_paths::hook_dir::PRE_COMMIT)
        .expect("pre-commit hook should be installed");
    assert_eq!(
        pre_commit_result.status,
        RequiredHookInstallStatus::Installed
    );

    let second_outcome =
        install::install_required_git_hooks(&repo).expect("second hook install should succeed");
    let second_pre_commit = second_outcome
        .hook_results
        .iter()
        .find(|result| result.hook_name == default_paths::hook_dir::PRE_COMMIT)
        .expect("pre-commit hook result should be present");
    assert_eq!(second_pre_commit.status, RequiredHookInstallStatus::Skipped);
    assert_eq!(
        fs::read(&second_pre_commit.hook_path).expect("read block-only pre-commit hook"),
        fs::read(&pre_commit_result.hook_path).expect("read initial pre-commit hook"),
        "block-only hook bytes should be unchanged across reruns"
    );

    let commit_msg_result = first_outcome
        .hook_results
        .iter()
        .find(|result| result.hook_name == default_paths::hook_dir::COMMIT_MSG)
        .expect("commit-msg hook should be installed");
    let commit_msg_path = commit_msg_result.hook_path.clone();
    let foreign_prefix = b"#!/bin/sh\necho foreign-commit-msg-guard\n".to_vec();
    fs::write(&commit_msg_path, &foreign_prefix).expect("seed foreign commit-msg hook");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&commit_msg_path, fs::Permissions::from_mode(0o755))
            .expect("mark foreign commit-msg hook executable");
    }

    let appended_outcome = install::install_required_git_hooks(&repo)
        .expect("hook install appending to foreign commit-msg hook should succeed");
    let appended_result = appended_outcome
        .hook_results
        .iter()
        .find(|result| result.hook_name == default_paths::hook_dir::COMMIT_MSG)
        .expect("commit-msg hook result should be present");
    assert_eq!(appended_result.status, RequiredHookInstallStatus::Updated);
    let appended_bytes = fs::read(&commit_msg_path).expect("read appended commit-msg hook");

    let rerun_outcome = install::install_required_git_hooks(&repo)
        .expect("rerunning hook install over foreign-plus-block hook should succeed");
    let rerun_result = rerun_outcome
        .hook_results
        .iter()
        .find(|result| result.hook_name == default_paths::hook_dir::COMMIT_MSG)
        .expect("commit-msg hook result should be present");
    assert_eq!(rerun_result.status, RequiredHookInstallStatus::Skipped);
    assert_eq!(
        fs::read(&commit_msg_path).expect("read commit-msg hook after rerun"),
        appended_bytes,
        "foreign-plus-block hook bytes should be unchanged across reruns"
    );

    let _ = fs::remove_dir_all(&repo);
}

#[test]
fn legacy_pre_marker_hook_upgrades_to_the_managed_block_form() {
    let repo = init_git_repo("hook-install-legacy-upgrade");

    let initial_outcome =
        install::install_required_git_hooks(&repo).expect("initial hook install should succeed");
    let pre_commit_path = initial_outcome
        .hook_results
        .iter()
        .find(|result| result.hook_name == default_paths::hook_dir::PRE_COMMIT)
        .expect("pre-commit hook should be installed")
        .hook_path
        .clone();
    let canonical_bytes = fs::read(&pre_commit_path).expect("read canonical pre-commit hook");

    let legacy_bytes = b"#!/bin/sh\nset -eu\nif ! command -v sce >/dev/null 2>&1; then\n  echo 'Install: https://sce.crocoder.dev/docs/getting-started#install-cli'\n  exit 0\nfi\nexec sce hooks pre-commit \"$@\"\n".to_vec();
    fs::write(&pre_commit_path, &legacy_bytes).expect("seed legacy pre-commit hook");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&pre_commit_path, fs::Permissions::from_mode(0o755))
            .expect("mark legacy pre-commit hook executable");
    }

    let outcome = install::install_required_git_hooks(&repo)
        .expect("hook install upgrading a legacy hook should succeed");
    let result = outcome
        .hook_results
        .iter()
        .find(|result| result.hook_name == default_paths::hook_dir::PRE_COMMIT)
        .expect("pre-commit hook result should be present");

    assert_eq!(result.status, RequiredHookInstallStatus::Updated);
    assert_eq!(
        fs::read(&pre_commit_path).expect("read upgraded pre-commit hook"),
        canonical_bytes,
        "a legacy pre-marker hook should upgrade to the canonical marker form"
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&pre_commit_path)
            .expect("stat upgraded pre-commit hook")
            .permissions()
            .mode();
        assert_ne!(mode & 0o111, 0, "upgraded hook should remain executable");
    }

    let _ = fs::remove_dir_all(&repo);
}

#[test]
fn foreign_hook_ending_in_exec_installs_the_block_and_reports_the_advisory() {
    let repo = init_git_repo("hook-install-unreachable-advisory");

    let initial_outcome =
        install::install_required_git_hooks(&repo).expect("initial hook install should succeed");
    let pre_commit_path = initial_outcome
        .hook_results
        .iter()
        .find(|result| result.hook_name == default_paths::hook_dir::PRE_COMMIT)
        .expect("pre-commit hook should be installed")
        .hook_path
        .clone();
    let commit_msg_path = initial_outcome
        .hook_results
        .iter()
        .find(|result| result.hook_name == default_paths::hook_dir::COMMIT_MSG)
        .expect("commit-msg hook should be installed")
        .hook_path
        .clone();

    let unreachable_foreign = b"#!/bin/sh\nexec some-other-tool \"$@\"\n".to_vec();
    fs::write(&pre_commit_path, &unreachable_foreign).expect("seed unreachable foreign hook");
    let ordinary_foreign = b"#!/bin/sh\necho foreign-commit-msg-guard\n".to_vec();
    fs::write(&commit_msg_path, &ordinary_foreign).expect("seed ordinary foreign hook");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&pre_commit_path, fs::Permissions::from_mode(0o755))
            .expect("mark unreachable foreign hook executable");
        fs::set_permissions(&commit_msg_path, fs::Permissions::from_mode(0o755))
            .expect("mark ordinary foreign hook executable");
    }

    let outcome = install::install_required_git_hooks(&repo)
        .expect("hook install over foreign hooks should succeed");

    let pre_commit_result = outcome
        .hook_results
        .iter()
        .find(|result| result.hook_name == default_paths::hook_dir::PRE_COMMIT)
        .expect("pre-commit hook result should be present");
    assert_eq!(pre_commit_result.status, RequiredHookInstallStatus::Updated);
    assert!(
        pre_commit_result.unreachable_block_advisory,
        "a hook ending in a zero-indent exec should report the advisory"
    );
    assert!(
        fs::read(&pre_commit_path)
            .expect("read pre-commit hook")
            .starts_with(&unreachable_foreign),
        "the block should still be installed even though it is unreachable"
    );

    let commit_msg_result = outcome
        .hook_results
        .iter()
        .find(|result| result.hook_name == default_paths::hook_dir::COMMIT_MSG)
        .expect("commit-msg hook result should be present");
    assert!(
        !commit_msg_result.unreachable_block_advisory,
        "a hook ending in an ordinary command should not report the advisory"
    );

    let _ = fs::remove_dir_all(&repo);
}
