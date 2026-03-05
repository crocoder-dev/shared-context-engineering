use anyhow::{bail, Result};
use lexopt::Arg;
use lexopt::ValueExt;

pub const NAME: &str = "completion";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompletionShell {
    Bash,
    Zsh,
    Fish,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompletionRequest {
    pub shell: CompletionShell,
}

pub fn completion_usage_text() -> &'static str {
    "Usage:\n  sce completion --shell <bash|zsh|fish>\n\nExamples:\n  sce completion --shell bash > ./sce.bash\n  sce completion --shell zsh > ./_sce\n  sce completion --shell fish > ~/.config/fish/completions/sce.fish"
}

pub fn parse_completion_request(args: Vec<String>) -> Result<CompletionRequest> {
    let mut parser = lexopt::Parser::from_args(args);
    let mut shell = None;

    while let Some(arg) = parser.next()? {
        match arg {
            Arg::Long("shell") => {
                if shell.is_some() {
                    bail!(
                        "Option '--shell' may only be provided once. Run 'sce completion --help' to see valid usage."
                    );
                }
                let value = parser.value()?;
                let raw = value.string()?;
                shell = Some(parse_shell(&raw)?);
            }
            Arg::Long("help") | Arg::Short('h') => {
                bail!("Use 'sce completion --help' for completion usage.");
            }
            Arg::Long(option) => {
                bail!(
                    "Unknown completion option '--{}'. Run 'sce completion --help' to see valid usage.",
                    option
                );
            }
            Arg::Short(option) => {
                bail!(
                    "Unknown completion option '-{}'. Run 'sce completion --help' to see valid usage.",
                    option
                );
            }
            Arg::Value(value) => {
                bail!(
                    "Unexpected completion argument '{}'. Run 'sce completion --help' to see valid usage.",
                    value.string()?
                );
            }
        }
    }

    let Some(shell) = shell else {
        bail!(
            "Missing required option '--shell <bash|zsh|fish>'. Run 'sce completion --help' to see valid usage."
        );
    };

    Ok(CompletionRequest { shell })
}

fn parse_shell(raw: &str) -> Result<CompletionShell> {
    match raw {
        "bash" => Ok(CompletionShell::Bash),
        "zsh" => Ok(CompletionShell::Zsh),
        "fish" => Ok(CompletionShell::Fish),
        _ => bail!(
            "Unsupported shell '{}'. Valid values: bash, zsh, fish.",
            raw
        ),
    }
}

pub fn render_completion(request: CompletionRequest) -> String {
    match request.shell {
        CompletionShell::Bash => bash_completion_script().to_string(),
        CompletionShell::Zsh => zsh_completion_script().to_string(),
        CompletionShell::Fish => fish_completion_script().to_string(),
    }
}

fn bash_completion_script() -> &'static str {
    r#"_sce_complete() {
  local cur prev cmd subcmd
  cur="${COMP_WORDS[COMP_CWORD]}"
  prev="${COMP_WORDS[COMP_CWORD-1]}"
  cmd="${COMP_WORDS[1]}"
  subcmd="${COMP_WORDS[2]}"

  if [[ ${COMP_CWORD} -eq 1 ]]; then
    COMPREPLY=( $(compgen -W "help config setup doctor mcp hooks sync version completion" -- "${cur}") )
    return
  fi

  case "${cmd}" in
    config)
      if [[ ${COMP_CWORD} -eq 2 ]]; then
        COMPREPLY=( $(compgen -W "show validate --help -h" -- "${cur}") )
        return
      fi
      if [[ "${prev}" == "--format" ]]; then
        COMPREPLY=( $(compgen -W "text json" -- "${cur}") )
        return
      fi
      if [[ "${prev}" == "--log-level" ]]; then
        COMPREPLY=( $(compgen -W "error warn info debug" -- "${cur}") )
        return
      fi
      COMPREPLY=( $(compgen -W "--config --log-level --timeout-ms --format --help -h" -- "${cur}") )
      ;;
    setup)
      if [[ "${prev}" == "--repo" ]]; then
        COMPREPLY=( $(compgen -d -- "${cur}") )
        return
      fi
      COMPREPLY=( $(compgen -W "--opencode --claude --both --non-interactive --hooks --repo --help -h" -- "${cur}") )
      ;;
    doctor)
      COMPREPLY=( $(compgen -W "--help -h" -- "${cur}") )
      ;;
    mcp)
      COMPREPLY=( $(compgen -W "--help -h" -- "${cur}") )
      ;;
    hooks)
      if [[ ${COMP_CWORD} -eq 2 ]]; then
        COMPREPLY=( $(compgen -W "pre-commit commit-msg post-commit post-rewrite --help -h" -- "${cur}") )
        return
      fi
      if [[ "${subcmd}" == "post-rewrite" && ${COMP_CWORD} -eq 3 ]]; then
        COMPREPLY=( $(compgen -W "amend rebase other" -- "${cur}") )
        return
      fi
      ;;
    sync)
      COMPREPLY=( $(compgen -W "--help -h" -- "${cur}") )
      ;;
    version)
      if [[ "${prev}" == "--format" ]]; then
        COMPREPLY=( $(compgen -W "text json" -- "${cur}") )
        return
      fi
      COMPREPLY=( $(compgen -W "--format --help -h" -- "${cur}") )
      ;;
    completion)
      if [[ "${prev}" == "--shell" ]]; then
        COMPREPLY=( $(compgen -W "bash zsh fish" -- "${cur}") )
        return
      fi
      COMPREPLY=( $(compgen -W "--shell --help -h" -- "${cur}") )
      ;;
    help)
      ;;
  esac
}

complete -F _sce_complete sce
"#
}

fn zsh_completion_script() -> &'static str {
    r#"#compdef sce

local -a commands
commands=(help config setup doctor mcp hooks sync version completion)

if (( CURRENT == 2 )); then
  compadd -- $commands
  return
fi

case "${words[2]}" in
  config)
    if (( CURRENT == 3 )); then
      compadd -- show validate --help -h
      return
    fi
    case "${words[CURRENT-1]}" in
      --format)
        compadd -- text json
        return
        ;;
      --log-level)
        compadd -- error warn info debug
        return
        ;;
    esac
    compadd -- --config --log-level --timeout-ms --format --help -h
    ;;
  setup)
    if [[ "${words[CURRENT-1]}" == "--repo" ]]; then
      _files -/
      return
    fi
    compadd -- --opencode --claude --both --non-interactive --hooks --repo --help -h
    ;;
  doctor)
    compadd -- --help -h
    ;;
  mcp)
    compadd -- --help -h
    ;;
  hooks)
    if (( CURRENT == 3 )); then
      compadd -- pre-commit commit-msg post-commit post-rewrite --help -h
      return
    fi
    if [[ "${words[3]}" == "post-rewrite" && CURRENT == 4 ]]; then
      compadd -- amend rebase other
      return
    fi
    ;;
  sync)
    compadd -- --help -h
    ;;
  version)
    if [[ "${words[CURRENT-1]}" == "--format" ]]; then
      compadd -- text json
      return
    fi
    compadd -- --format --help -h
    ;;
  completion)
    if [[ "${words[CURRENT-1]}" == "--shell" ]]; then
      compadd -- bash zsh fish
      return
    fi
    compadd -- --shell --help -h
    ;;
esac
"#
}

fn fish_completion_script() -> &'static str {
    r#"complete -c sce -f

complete -c sce -n "__fish_use_subcommand" -a "help config setup doctor mcp hooks sync version completion"

complete -c sce -n "__fish_seen_subcommand_from config" -a "show validate"
complete -c sce -n "__fish_seen_subcommand_from config" -l config -r
complete -c sce -n "__fish_seen_subcommand_from config" -l log-level -r -a "error warn info debug"
complete -c sce -n "__fish_seen_subcommand_from config" -l timeout-ms -r
complete -c sce -n "__fish_seen_subcommand_from config" -l format -r -a "text json"

complete -c sce -n "__fish_seen_subcommand_from setup" -l opencode
complete -c sce -n "__fish_seen_subcommand_from setup" -l claude
complete -c sce -n "__fish_seen_subcommand_from setup" -l both
complete -c sce -n "__fish_seen_subcommand_from setup" -l non-interactive
complete -c sce -n "__fish_seen_subcommand_from setup" -l hooks
complete -c sce -n "__fish_seen_subcommand_from setup" -l repo -r -a "(__fish_complete_directories)"

complete -c sce -n "__fish_seen_subcommand_from hooks" -a "pre-commit commit-msg post-commit post-rewrite"
complete -c sce -n "__fish_seen_subcommand_from hooks post-rewrite" -a "amend rebase other"

complete -c sce -n "__fish_seen_subcommand_from version" -l format -r -a "text json"

complete -c sce -n "__fish_seen_subcommand_from completion" -l shell -r -a "bash zsh fish"
"#
}

#[cfg(test)]
mod tests {
    use super::{parse_completion_request, render_completion, CompletionRequest, CompletionShell};

    #[test]
    fn parse_requires_shell() {
        let error = parse_completion_request(vec![]).expect_err("missing --shell should fail");
        assert!(error
            .to_string()
            .contains("Missing required option '--shell"));
    }

    #[test]
    fn parse_accepts_shell_value() {
        let request = parse_completion_request(vec!["--shell".to_string(), "zsh".to_string()])
            .expect("request should parse");
        assert_eq!(request.shell, CompletionShell::Zsh);
    }

    #[test]
    fn parse_rejects_duplicate_shell_option() {
        let error = parse_completion_request(vec![
            "--shell".to_string(),
            "bash".to_string(),
            "--shell".to_string(),
            "zsh".to_string(),
        ])
        .expect_err("duplicate --shell should fail");
        assert!(error
            .to_string()
            .contains("Option '--shell' may only be provided once"));
    }

    #[test]
    fn render_bash_completion_is_deterministic() {
        let output = render_completion(CompletionRequest {
            shell: CompletionShell::Bash,
        });
        assert!(output.contains("complete -F _sce_complete sce"));
        assert!(output.contains("help config setup doctor mcp hooks sync version completion"));
    }

    #[test]
    fn render_zsh_completion_has_compdef_header() {
        let output = render_completion(CompletionRequest {
            shell: CompletionShell::Zsh,
        });
        assert!(output.contains("#compdef sce"));
        assert!(output.contains("completion"));
    }

    #[test]
    fn render_fish_completion_has_completion_command() {
        let output = render_completion(CompletionRequest {
            shell: CompletionShell::Fish,
        });
        assert!(output.contains("complete -c sce -f"));
        assert!(output.contains("completion"));
    }
}
