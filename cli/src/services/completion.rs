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
