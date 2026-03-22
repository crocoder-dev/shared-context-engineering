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

pub fn style_if_enabled<F>(text: &str, f: F) -> String
where
    F: FnOnce(&str) -> String,
{
    if supports_color() {
        f(text)
    } else {
        text.to_string()
    }
}

#[must_use]
pub fn heading(text: &str) -> String {
    style_if_enabled(text, |s| s.cyan().bold().to_string())
}

#[must_use]
pub fn command_name(text: &str) -> String {
    style_if_enabled(text, |s| s.green().to_string())
}

#[must_use]
pub fn error_code(text: &str) -> String {
    if supports_color_stderr() {
        text.red().bold().to_string()
    } else {
        text.to_string()
    }
}

#[must_use]
#[allow(dead_code)]
pub fn example_command(text: &str) -> String {
    style_if_enabled(text, |s| s.yellow().to_string())
}

#[must_use]
#[allow(dead_code)]
pub fn placeholder(text: &str) -> String {
    style_if_enabled(text, |s| s.italic().dimmed().to_string())
}

#[must_use]
pub fn status_implemented(text: &str) -> String {
    style_if_enabled(text, |s| s.green().to_string())
}

#[must_use]
pub fn status_placeholder(text: &str) -> String {
    style_if_enabled(text, |s| s.dimmed().to_string())
}

#[must_use]
#[allow(dead_code)]
pub fn heading_stderr(text: &str) -> String {
    if supports_color_stderr() {
        text.cyan().bold().to_string()
    } else {
        text.to_string()
    }
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
    style_if_enabled(text, |s| s.bold().to_string())
}

#[must_use]
pub fn prompt_value(text: &str) -> String {
    style_if_enabled(text, |s| s.yellow().to_string())
}

#[cfg(test)]
mod tests;
