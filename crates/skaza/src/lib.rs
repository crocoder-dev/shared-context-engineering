pub mod mutation;
pub mod report;
pub mod rust;
pub mod source;

use std::num::NonZeroUsize;
use std::path::PathBuf;

use clap::{Parser, ValueEnum};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub enum Format {
    #[default]
    Text,
    Json,
}

#[derive(Clone, Debug, Eq, PartialEq, Parser)]
#[command(
    name = "skaza",
    version,
    about = "Discover Rust mutation candidates without applying or executing them"
)]
pub struct Args {
    #[arg(
        long = "src",
        value_name = "PATH",
        default_value = ".",
        help = "Rust source file or directory to scan; repeat to scan several"
    )]
    pub src: Vec<PathBuf>,

    #[arg(
        long,
        value_name = "N",
        default_value = "10",
        value_parser = parse_limit,
        help = "Maximum number of candidates to report (must be at least 1)"
    )]
    pub limit: NonZeroUsize,

    #[arg(
        long,
        value_enum,
        default_value_t = Format::Text,
        help = "Report format"
    )]
    pub format: Format,
}

fn parse_limit(value: &str) -> Result<NonZeroUsize, String> {
    let parsed: usize = value.parse().map_err(|_| {
        format!("`{value}` is not a positive integer; pass a value such as `--limit 10`")
    })?;
    NonZeroUsize::new(parsed).ok_or_else(|| {
        "limit must be at least 1; pass a positive value such as `--limit 10`".to_string()
    })
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroUsize;
    use std::path::PathBuf;

    use clap::error::ErrorKind;
    use clap::Parser;

    use super::{Args, Format};

    #[test]
    fn defaults_scan_current_directory_with_limit_ten_in_text() {
        let args = Args::try_parse_from(["skaza"]).unwrap();
        assert_eq!(
            args,
            Args {
                src: vec![PathBuf::from(".")],
                limit: NonZeroUsize::new(10).unwrap(),
                format: Format::Text,
            }
        );
    }

    #[test]
    fn repeated_src_values_are_kept_in_order() {
        let args =
            Args::try_parse_from(["skaza", "--src", "a.rs", "--src", "dir", "--src", "a.rs"])
                .unwrap();
        assert_eq!(
            args.src,
            vec![
                PathBuf::from("a.rs"),
                PathBuf::from("dir"),
                PathBuf::from("a.rs"),
            ]
        );
    }

    #[test]
    fn explicit_limit_and_json_format_are_accepted() {
        let args = Args::try_parse_from(["skaza", "--limit", "3", "--format", "json"]).unwrap();
        assert_eq!(args.limit.get(), 3);
        assert_eq!(args.format, Format::Json);
    }

    #[test]
    fn zero_limit_is_rejected_with_actionable_error() {
        let error = Args::try_parse_from(["skaza", "--limit", "0"]).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::ValueValidation);
        assert!(error
            .to_string()
            .contains("limit must be at least 1; pass a positive value such as `--limit 10`"));
    }

    #[test]
    fn non_numeric_limit_is_rejected() {
        let error = Args::try_parse_from(["skaza", "--limit", "many"]).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::ValueValidation);
    }

    #[test]
    fn invalid_format_is_rejected() {
        let error = Args::try_parse_from(["skaza", "--format", "sarif"]).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidValue);
    }

    #[test]
    fn subcommands_are_not_accepted() {
        let error = Args::try_parse_from(["skaza", "run"]).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::UnknownArgument);
    }
}
