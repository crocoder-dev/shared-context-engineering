use crate::channels::Channel;

const USAGE: &str = "Usage: install-channel-integration-tests [--channel <npm|bun|cargo|all>]\n\nOpt-in install-channel integration runner for `sce`.\nThe npm and Bun channels now perform real install-and-verify flows through the\nRust runner, while Cargo remains a shared-harness smoke path until a later task.\n\nSelectors:\n  npm    Run the npm install-and-verify channel path\n  bun    Run the Bun install-and-verify channel path\n  cargo  Run only the Cargo channel path\n  all    Run all channel paths (default)";

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Command {
    Help(String),
    Run { channels: Vec<Channel> },
}

pub(crate) fn parse_args<I>(args: I) -> Result<Command, String>
where
    I: IntoIterator<Item = String>,
{
    let mut channel_selector = String::from("all");
    let mut args = args.into_iter();

    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--help" | "-h" => return Ok(Command::Help(USAGE.to_string())),
            "--channel" => {
                let selector = args
                    .next()
                    .ok_or_else(|| String::from("Missing value for --channel.\n\nTry --channel npm, --channel bun, --channel cargo, or --channel all."))?;
                channel_selector = selector;
            }
            _ => {
                return Err(format!("Unknown argument: {argument}\n\n{USAGE}"));
            }
        }
    }

    let channels = Channel::from_selector(&channel_selector)?;
    Ok(Command::Run { channels })
}
