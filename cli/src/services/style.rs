#[allow(unused_imports)]
pub use owo_colors::OwoColorize;

use std::io::IsTerminal;

#[must_use]
pub fn supports_color() -> bool {
    if std::env::var_os("NO_COLOR").is_some() {
        return false;
    }

    std::io::stdout().is_terminal()
}

#[must_use]
pub fn supports_color_stderr() -> bool {
    if std::env::var_os("NO_COLOR").is_some() {
        return false;
    }

    std::io::stderr().is_terminal()
}

fn style_if<F>(text: &str, color_enabled: bool, f: F) -> String
where
    F: FnOnce(&str) -> String,
{
    if color_enabled {
        f(text)
    } else {
        text.to_string()
    }
}

pub fn style_if_enabled<F>(text: &str, f: F) -> String
where
    F: FnOnce(&str) -> String,
{
    style_if(text, supports_color(), f)
}

pub(crate) fn style_if_enabled_stderr<F>(text: &str, f: F) -> String
where
    F: FnOnce(&str) -> String,
{
    style_if(text, supports_color_stderr(), f)
}

#[must_use]
pub fn heading(text: &str) -> String {
    heading_with_color_policy(text, supports_color())
}

#[must_use]
fn heading_with_color_policy(text: &str, color_enabled: bool) -> String {
    style_if(text, color_enabled, |s| s.cyan().bold().to_string())
}

#[must_use]
pub fn command_name(text: &str) -> String {
    command_name_with_color_policy(text, supports_color())
}

#[must_use]
fn command_name_with_color_policy(text: &str, color_enabled: bool) -> String {
    style_if(text, color_enabled, |s| s.green().to_string())
}

#[must_use]
pub fn error_code(text: &str) -> String {
    style_if_enabled_stderr(text, |s| s.red().bold().to_string())
}

#[must_use]
pub fn error_text(text: &str) -> String {
    style_if_enabled_stderr(text, |s| s.yellow().to_string())
}

#[must_use]
fn placeholder_with_color_policy(text: &str, color_enabled: bool) -> String {
    style_if(text, color_enabled, |s| s.italic().dimmed().to_string())
}

#[must_use]
pub fn success(text: &str) -> String {
    style_if_enabled(text, |s| s.green().bold().to_string())
}

#[must_use]
pub fn label(text: &str) -> String {
    style_if_enabled(text, |s| s.cyan().to_string())
}

#[must_use]
pub fn value(text: &str) -> String {
    text.to_string()
}

#[must_use]
pub fn prompt_label(text: &str) -> String {
    prompt_label_with_color_policy(text, supports_color())
}

#[must_use]
pub fn prompt_value(text: &str) -> String {
    prompt_value_with_color_policy(text, supports_color())
}

#[must_use]
pub(crate) fn prompt_label_with_color_policy(text: &str, color_enabled: bool) -> String {
    style_if(text, color_enabled, |s| s.bold().to_string())
}

#[must_use]
pub(crate) fn prompt_value_with_color_policy(text: &str, color_enabled: bool) -> String {
    style_if(text, color_enabled, |s| s.yellow().to_string())
}

#[must_use]
pub fn clap_help(text: &str) -> String {
    clap_help_with_color_policy(text, supports_color())
}

#[must_use]
pub(crate) fn clap_help_with_color_policy(text: &str, color_enabled: bool) -> String {
    if !color_enabled {
        return text.to_string();
    }

    let trailing_newline = text.ends_with('\n');
    let rendered = text
        .lines()
        .map(|line| style_clap_help_line(line, color_enabled))
        .collect::<Vec<_>>()
        .join("\n");

    if trailing_newline {
        format!("{rendered}\n")
    } else {
        rendered
    }
}

fn style_clap_help_line(line: &str, color_enabled: bool) -> String {
    if line.is_empty() {
        return String::new();
    }

    if let Some(remainder) = line.strip_prefix("Usage: ") {
        return format!(
            "{} {}",
            heading_with_color_policy("Usage:", color_enabled),
            style_clap_usage_segment(remainder, color_enabled)
        );
    }

    if !line.starts_with(' ') && line.ends_with(':') {
        return heading_with_color_policy(line, color_enabled);
    }

    if let Some((indent, token, remainder)) = split_help_table_row(line) {
        let styled_token = if token.starts_with('-') {
            style_help_placeholders(token, color_enabled)
        } else {
            command_name_with_color_policy(token, color_enabled)
        };

        return format!("{indent}{styled_token}{remainder}");
    }

    style_help_placeholders(line, color_enabled)
}

fn split_help_table_row(line: &str) -> Option<(&str, &str, &str)> {
    if !line.starts_with("  ") {
        return None;
    }

    let trimmed = line.trim_start();
    let indent_len = line.len() - trimmed.len();
    let token_end = trimmed.find("  ")?;
    let token = &trimmed[..token_end];
    let remainder = &trimmed[token_end..];

    Some((&line[..indent_len], token, remainder))
}

fn style_clap_usage_segment(segment: &str, color_enabled: bool) -> String {
    segment
        .split(' ')
        .map(|part| {
            if part.is_empty() {
                String::new()
            } else if is_help_placeholder(part) {
                style_help_placeholders(part, color_enabled)
            } else {
                command_name_with_color_policy(part, color_enabled)
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn style_help_placeholders(text: &str, color_enabled: bool) -> String {
    let mut styled = String::new();
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '<' || ch == '[' {
            let closing = if ch == '<' { '>' } else { ']' };
            let mut token = String::from(ch);
            let mut closed = false;

            for next in chars.by_ref() {
                token.push(next);
                if next == closing {
                    closed = true;
                    break;
                }
            }

            if closed && is_help_placeholder(&token) {
                styled.push_str(&placeholder_with_color_policy(&token, color_enabled));
            } else {
                styled.push_str(&token);
            }
        } else {
            styled.push(ch);
        }
    }

    styled
}

fn is_help_placeholder(token: &str) -> bool {
    let Some(inner) = token
        .strip_prefix('<')
        .and_then(|s| s.strip_suffix('>'))
        .or_else(|| token.strip_prefix('[').and_then(|s| s.strip_suffix(']')))
    else {
        return false;
    };

    inner
        .chars()
        .any(|ch| ch.is_ascii_uppercase() || matches!(ch, '<' | '>' | '[' | ']'))
}

/// Gradient color endpoints for the ASCII art banner.
/// Right side: cyan (0, 255, 255), Left side: magenta (255, 0, 255).
#[allow(dead_code)]
const GRADIENT_COLOR_RIGHT: (u8, u8, u8) = (0, 255, 255);
#[allow(dead_code)]
const GRADIENT_COLOR_LEFT: (u8, u8, u8) = (255, 0, 255);

/// Applies a right-to-left color gradient to ASCII art banner lines.
///
/// When color is enabled, each non-space character is colored based on its
/// column position using a cyan-to-magenta gradient (cyan on the right,
/// magenta on the left). Spaces are left unstyled.
/// When color is disabled, returns the plain ASCII art unchanged.
#[allow(dead_code)]
#[must_use]
pub fn banner_with_gradient(lines: &[&str]) -> String {
    banner_with_gradient_with_color_policy(lines, supports_color())
}

/// Internal variant of [`banner_with_gradient`] that accepts an explicit
/// color policy flag for testability.
#[allow(dead_code, clippy::cast_precision_loss)]
#[must_use]
pub(crate) fn banner_with_gradient_with_color_policy(
    lines: &[&str],
    color_enabled: bool,
) -> String {
    if !color_enabled {
        return lines.join("\n");
    }

    let max_width = lines
        .iter()
        .map(|line| line.chars().count())
        .max()
        .unwrap_or(0);
    if max_width == 0 {
        return lines.join("\n");
    }

    let max_col = (max_width.saturating_sub(1) as f64).max(1.0);

    lines
        .iter()
        .map(|line| {
            line.chars()
                .enumerate()
                .map(|(col, ch)| {
                    if ch == ' ' {
                        ch.to_string()
                    } else {
                        let t = col as f64 / max_col;
                        let r = lerp_u8(GRADIENT_COLOR_LEFT.0, GRADIENT_COLOR_RIGHT.0, t);
                        let g = lerp_u8(GRADIENT_COLOR_LEFT.1, GRADIENT_COLOR_RIGHT.1, t);
                        let b = lerp_u8(GRADIENT_COLOR_LEFT.2, GRADIENT_COLOR_RIGHT.2, t);
                        ch.truecolor(r, g, b).to_string()
                    }
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Linear interpolation between two `u8` values.
///
/// Returns `a + (b - a) * t`, clamped to the `u8` range by construction
/// since `a`, `b` are `u8` and `t` is in `[0, 1]`.
#[allow(dead_code, clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn lerp_u8(a: u8, b: u8, t: f64) -> u8 {
    (f64::from(a) + (f64::from(b) - f64::from(a)) * t).round() as u8
}
