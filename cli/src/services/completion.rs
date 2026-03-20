pub const NAME: &str = "completion";

use clap::CommandFactory;
use clap_complete::Shell;

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

pub fn render_completion(request: CompletionRequest) -> String {
    let shell = match request.shell {
        CompletionShell::Bash => Shell::Bash,
        CompletionShell::Zsh => Shell::Zsh,
        CompletionShell::Fish => Shell::Fish,
    };

    let mut buffer = Vec::new();
    clap_complete::generate(
        shell,
        &mut crate::cli_schema::Cli::command(),
        "sce",
        &mut buffer,
    );

    String::from_utf8(buffer).expect("Generated completion script should be valid UTF-8")
}

#[cfg(test)]
mod tests {
    use super::{render_completion, CompletionRequest, CompletionShell};

    #[test]
    fn render_bash_completion_is_deterministic() {
        let output = render_completion(CompletionRequest {
            shell: CompletionShell::Bash,
        });
        assert!(output.contains("_sce()"));
        assert!(output.contains("COMPREPLY"));
        assert!(output.contains("config"));
        assert!(output.contains("setup"));
        assert!(output.contains("completion"));
    }

    #[test]
    fn render_zsh_completion_has_compdef_header() {
        let output = render_completion(CompletionRequest {
            shell: CompletionShell::Zsh,
        });
        assert!(output.contains("#compdef sce"));
        assert!(output.contains("config"));
        assert!(output.contains("completion"));
    }

    #[test]
    fn render_fish_completion_has_completion_command() {
        let output = render_completion(CompletionRequest {
            shell: CompletionShell::Fish,
        });
        assert!(output.contains("complete -c sce"));
        assert!(output.contains("config"));
        assert!(output.contains("completion"));
    }

    #[test]
    fn completion_includes_all_commands() {
        let output = render_completion(CompletionRequest {
            shell: CompletionShell::Bash,
        });
        assert!(output.contains("config"));
        assert!(output.contains("setup"));
        assert!(output.contains("doctor"));
        assert!(output.contains("hooks"));
        assert!(output.contains("sync"));
        assert!(output.contains("version"));
        assert!(output.contains("completion"));
    }
}
