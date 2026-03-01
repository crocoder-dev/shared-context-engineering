use std::process::ExitCode;

use crate::{command_surface, dependency_contract};

pub fn run<I>(args: I) -> ExitCode
where
    I: IntoIterator<Item = String>,
{
    let _ = dependency_contract::dependency_contract_snapshot();

    let mut args = args.into_iter();
    let _program = args.next();

    match args.next().as_deref() {
        None | Some("--help") | Some("-h") | Some("help") => {
            println!("{}", command_surface::help_text());
            ExitCode::SUCCESS
        }
        Some(cmd) => {
            if command_surface::is_known_command(cmd) {
                println!(
                    "'{}' is a planned placeholder command in this foundation slice.",
                    cmd
                );
                ExitCode::SUCCESS
            } else {
                eprintln!("Unknown command: {}", cmd);
                eprintln!("Run 'sce --help' to see the current placeholder command surface.");
                ExitCode::from(2)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::process::ExitCode;

    use super::run;

    #[test]
    fn help_path_exits_success() {
        let code = run(vec!["sce".to_string(), "--help".to_string()]);
        assert_eq!(code, ExitCode::SUCCESS);
    }

    #[test]
    fn placeholder_command_exits_success() {
        let code = run(vec!["sce".to_string(), "setup".to_string()]);
        assert_eq!(code, ExitCode::SUCCESS);
    }

    #[test]
    fn unknown_command_exits_non_zero() {
        let code = run(vec!["sce".to_string(), "does-not-exist".to_string()]);
        assert_eq!(code, ExitCode::from(2));
    }
}
