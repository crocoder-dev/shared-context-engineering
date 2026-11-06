use std::fmt;

const QUOTE: u8 = b'"';
const BACKSLASH: u8 = b'\\';
const GIT_SOURCE_PREFIX: &str = "a/";
const GIT_DESTINATION_PREFIX: &str = "b/";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GitPathError {
    UnterminatedQuote,
    InvalidEscape(String),
    NulByte,
    InvalidUtf8,
    TrailingContent(String),
    MissingSecondPath,
}

impl fmt::Display for GitPathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnterminatedQuote => write!(f, "quoted path has no closing quote"),
            Self::InvalidEscape(escape) => {
                write!(f, "quoted path contains invalid escape sequence {escape:?}")
            }
            Self::NulByte => write!(f, "quoted path decodes to a NUL byte"),
            Self::InvalidUtf8 => write!(f, "quoted path does not decode to valid UTF-8"),
            Self::TrailingContent(rest) => {
                write!(f, "unexpected content {rest:?} after quoted path")
            }
            Self::MissingSecondPath => write!(f, "header does not name two paths"),
        }
    }
}

impl std::error::Error for GitPathError {}

pub(super) fn read_quoted(input: &str) -> Result<(String, &str), GitPathError> {
    let bytes = input.as_bytes();
    debug_assert_eq!(bytes.first(), Some(&QUOTE));
    let mut decoded = Vec::new();
    let mut index = 1;
    while let Some(&byte) = bytes.get(index) {
        match byte {
            QUOTE => return Ok((into_path(decoded)?, &input[index + 1..])),
            BACKSLASH => {
                let (value, width) = unescape(&bytes[index + 1..])?;
                decoded.push(value);
                index += 1 + width;
            }
            _ => {
                decoded.push(byte);
                index += 1;
            }
        }
    }
    Err(GitPathError::UnterminatedQuote)
}

fn unescape(after_backslash: &[u8]) -> Result<(u8, usize), GitPathError> {
    let simple = match after_backslash.first() {
        Some(b'a') => Some(0x07),
        Some(b'b') => Some(0x08),
        Some(b't') => Some(b'\t'),
        Some(b'n') => Some(b'\n'),
        Some(b'v') => Some(0x0b),
        Some(b'f') => Some(0x0c),
        Some(b'r') => Some(b'\r'),
        Some(&QUOTE) => Some(QUOTE),
        Some(&BACKSLASH) => Some(BACKSLASH),
        _ => None,
    };
    if let Some(value) = simple {
        return Ok((value, 1));
    }
    if let [high @ b'0'..=b'3', mid @ b'0'..=b'7', low @ b'0'..=b'7', ..] = after_backslash {
        return Ok(((high - b'0') << 6 | (mid - b'0') << 3 | (low - b'0'), 3));
    }
    let shown = after_backslash.iter().take(3).copied().collect::<Vec<_>>();
    Err(GitPathError::InvalidEscape(format!(
        "\\{}",
        String::from_utf8_lossy(&shown)
    )))
}

fn into_path(bytes: Vec<u8>) -> Result<String, GitPathError> {
    if bytes.contains(&0) {
        return Err(GitPathError::NulByte);
    }
    String::from_utf8(bytes).map_err(|_| GitPathError::InvalidUtf8)
}

pub(super) fn decode_path(token: &str) -> Result<String, GitPathError> {
    if !token.starts_with('"') {
        return Ok(token.to_owned());
    }
    match read_quoted(token)? {
        (path, "") => Ok(path),
        (_, rest) => Err(GitPathError::TrailingContent(rest.to_owned())),
    }
}

pub(super) fn decode_marker_path(rest: &str) -> Result<String, GitPathError> {
    if rest.starts_with('"') {
        let (path, after) = read_quoted(rest)?;
        if after.is_empty() || after.starts_with('\t') {
            return Ok(path);
        }
        return Err(GitPathError::TrailingContent(after.to_owned()));
    }
    Ok(rest.split('\t').next().unwrap_or_default().to_owned())
}

pub(super) fn split_diff_header(rest: &str) -> Result<Option<(String, String)>, GitPathError> {
    if rest.starts_with('"') {
        let (old, after) = read_quoted(rest)?;
        let new = after
            .strip_prefix(' ')
            .ok_or(GitPathError::MissingSecondPath)?;
        return Ok(Some((old, decode_path(new)?)));
    }
    if rest.contains('"') {
        let index = rest.find(" \"").ok_or(GitPathError::MissingSecondPath)?;
        return Ok(Some((
            rest[..index].to_owned(),
            decode_path(&rest[index + 1..])?,
        )));
    }
    if let Some((old, new)) = equal_split(rest) {
        return Ok(Some((old.to_owned(), new.to_owned())));
    }
    let mut splits = rest.match_indices(" b/").map(|(index, _)| index);
    match (splits.next(), splits.next()) {
        (Some(index), None) => Ok(Some((
            rest[..index].to_owned(),
            rest[index + 1..].to_owned(),
        ))),
        (Some(_), Some(_)) => Ok(None),
        (None, _) => Err(GitPathError::MissingSecondPath),
    }
}

fn equal_split(rest: &str) -> Option<(&str, &str)> {
    if rest.len().is_multiple_of(2) {
        return None;
    }
    let middle = rest.len() / 2;
    if rest.as_bytes()[middle] != b' ' {
        return None;
    }
    let (old, new) = (&rest[..middle], &rest[middle + 1..]);
    let unprefixed_old = old.strip_prefix(GIT_SOURCE_PREFIX).unwrap_or(old);
    let unprefixed_new = new.strip_prefix(GIT_DESTINATION_PREFIX).unwrap_or(new);
    (unprefixed_old == unprefixed_new).then_some((old, new))
}

#[cfg(test)]
mod tests {
    use super::{decode_marker_path, decode_path, split_diff_header, GitPathError};

    #[test]
    fn unquoted_paths_are_literal() {
        for path in [
            "src/lib.rs",
            "src/file name.rs",
            "src/trailing  ",
            "a/b c/d",
        ] {
            assert_eq!(decode_path(path).as_deref(), Ok(path));
        }
    }

    #[test]
    fn quoted_paths_decode_every_escape_git_writes() {
        let cases = [
            (r#""a/src/caf\303\251.rs""#, "a/src/café.rs"),
            (r#""a/src/file\tname.rs""#, "a/src/file\tname.rs"),
            (r#""a/src/file\nname.rs""#, "a/src/file\nname.rs"),
            (r#""a/src/file\"name.rs""#, "a/src/file\"name.rs"),
            (r#""a/src/file\\name.rs""#, "a/src/file\\name.rs"),
            (r#""a/\a\b\v\f\r""#, "a/\x07\x08\x0b\x0c\r"),
            (r#""a/\001\177""#, "a/\x01\x7f"),
            (r#""a/\360\237\246\200 crab""#, "a/🦀 crab"),
            ("\"a/raw café\"", "a/raw café"),
        ];
        for (quoted, expected) in cases {
            assert_eq!(decode_path(quoted).as_deref(), Ok(expected), "{quoted}");
        }
    }

    #[test]
    fn octal_escapes_are_bytes_not_code_points() {
        assert_eq!(
            decode_path(r#""caf\351.rs""#),
            Err(GitPathError::InvalidUtf8)
        );
        assert_eq!(decode_path(r#""\303""#), Err(GitPathError::InvalidUtf8));
    }

    #[test]
    fn malformed_quoted_paths_are_rejected() {
        assert_eq!(
            decode_path(r#""src/lib.rs"#),
            Err(GitPathError::UnterminatedQuote)
        );
        assert_eq!(
            decode_path(r#""src/lib.rs\""#),
            Err(GitPathError::UnterminatedQuote)
        );
        for invalid in [r#""\q""#, r#""\400""#, r#""\38""#, r#""\12""#, r#""\""#] {
            assert!(
                matches!(
                    decode_path(invalid),
                    Err(GitPathError::InvalidEscape(_) | GitPathError::UnterminatedQuote)
                ),
                "{invalid}"
            );
        }
        assert!(matches!(
            decode_path(r#""\q""#),
            Err(GitPathError::InvalidEscape(_))
        ));
        assert_eq!(decode_path(r#""a\000b""#), Err(GitPathError::NulByte));
        assert_eq!(
            decode_path(r#""a"b"#),
            Err(GitPathError::TrailingContent(String::from("b")))
        );
    }

    #[test]
    fn marker_paths_drop_only_the_tab_suffix() {
        assert_eq!(
            decode_marker_path("a/src/file name.rs\t").as_deref(),
            Ok("a/src/file name.rs")
        );
        assert_eq!(
            decode_marker_path("a/src/trailing \t").as_deref(),
            Ok("a/src/trailing ")
        );
        assert_eq!(decode_marker_path("a/src/x  ").as_deref(), Ok("a/src/x  "));
        assert_eq!(
            decode_marker_path("src/x\t(revision 12)").as_deref(),
            Ok("src/x")
        );
        assert_eq!(
            decode_marker_path(concat!(r#""b/src/caf\303\251 x.rs""#, "\t")).as_deref(),
            Ok("b/src/café x.rs")
        );
        assert!(decode_marker_path(r#""b/x" y"#).is_err());
    }

    fn split(rest: &str) -> Option<(String, String)> {
        split_diff_header(rest).expect("header splits")
    }

    #[test]
    fn header_splits_respect_quoting_and_identity() {
        let pair = |old: &str, new: &str| Some((old.to_owned(), new.to_owned()));
        assert_eq!(
            split("a/src/lib.rs b/src/lib.rs"),
            pair("a/src/lib.rs", "b/src/lib.rs")
        );
        assert_eq!(
            split("a/src/x b/y.rs b/src/x b/y.rs"),
            pair("a/src/x b/y.rs", "b/src/x b/y.rs")
        );
        assert_eq!(
            split("a/src/trailing  b/src/trailing "),
            pair("a/src/trailing ", "b/src/trailing ")
        );
        assert_eq!(
            split(r#""a/src/caf\303\251.rs" "b/src/caf\303\251.rs""#),
            pair("a/src/café.rs", "b/src/café.rs")
        );
        assert_eq!(
            split(r#""a/src/x\" b/y" "b/src/x\" b/y""#),
            pair("a/src/x\" b/y", "b/src/x\" b/y")
        );
        assert_eq!(
            split(r#"a/old name.rs "b/caf\303\251.rs""#),
            pair("a/old name.rs", "b/café.rs")
        );
        assert_eq!(
            split(r#""a/caf\303\251.rs" b/new name.rs"#),
            pair("a/café.rs", "b/new name.rs")
        );
        assert_eq!(split("a/old.rs b/new.rs"), pair("a/old.rs", "b/new.rs"));
    }

    #[test]
    fn ambiguous_or_incomplete_headers_are_not_guessed() {
        assert_eq!(split("a/x b/y b/z"), None);
        assert_eq!(
            split_diff_header("a/only-one-path"),
            Err(GitPathError::MissingSecondPath)
        );
        assert_eq!(
            split_diff_header(r#""a/x""#),
            Err(GitPathError::MissingSecondPath)
        );
        assert!(split_diff_header(r#""a/\xyz" "b/x""#).is_err());
    }
}
