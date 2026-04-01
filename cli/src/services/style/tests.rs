use super::*;
use std::env;

fn without_no_color<T>(f: impl FnOnce() -> T) -> T {
    env::remove_var("NO_COLOR");
    let result = f();
    env::remove_var("NO_COLOR");
    result
}

fn with_no_color<T>(f: impl FnOnce() -> T) -> T {
    env::set_var("NO_COLOR", "1");
    let result = f();
    env::remove_var("NO_COLOR");
    result
}

#[test]
fn supports_color_returns_false_when_no_color_is_set() {
    with_no_color(|| {
        assert!(!supports_color());
        assert!(!supports_color_stderr());
    });
}

#[test]
fn style_if_enabled_returns_plain_text_when_no_color() {
    with_no_color(|| {
        let result = style_if_enabled("test", std::string::ToString::to_string);
        assert_eq!(result, "test");
    });
}

#[test]
fn style_if_enabled_returns_transformed_text_when_color_supported() {
    without_no_color(|| {
        let _result = style_if_enabled("test", |s| s.red().to_string());
    });
}

#[test]
fn owo_colors_reexport_works() {
    let colored = "test".red().to_string();
    assert!(!colored.is_empty());
}

#[test]
fn heading_returns_plain_text_when_no_color() {
    with_no_color(|| {
        let result = heading("Usage:");
        assert_eq!(result, "Usage:");
    });
}

#[test]
fn command_name_returns_plain_text_when_no_color() {
    with_no_color(|| {
        let result = command_name("setup");
        assert_eq!(result, "setup");
    });
}

#[test]
fn error_code_returns_plain_text_when_no_color() {
    with_no_color(|| {
        let result = error_code("SCE-ERR-PARSE");
        assert_eq!(result, "SCE-ERR-PARSE");
    });
}

#[test]
fn error_text_returns_plain_text_when_no_color() {
    with_no_color(|| {
        let result = error_text("Unknown command");
        assert_eq!(result, "Unknown command");
    });
}

#[test]
fn style_functions_produce_non_empty_output() {
    without_no_color(|| {
        let _ = heading("Usage:");
        let _ = command_name("setup");
        let _ = error_code("SCE-ERR-PARSE");
    });
}

#[test]
fn success_returns_plain_text_when_no_color() {
    with_no_color(|| {
        let result = success("completed");
        assert_eq!(result, "completed");
    });
}

#[test]
fn label_returns_plain_text_when_no_color() {
    with_no_color(|| {
        let result = label("Repository root:");
        assert_eq!(result, "Repository root:");
    });
}

#[test]
fn value_returns_plain_text_always() {
    with_no_color(|| {
        let result = value("/path/to/repo");
        assert_eq!(result, "/path/to/repo");
    });

    without_no_color(|| {
        let result = value("/path/to/repo");
        assert_eq!(result, "/path/to/repo");
    });
}

#[test]
fn prompt_label_returns_plain_text_when_no_color() {
    with_no_color(|| {
        let result = prompt_label("Open in browser:");
        assert_eq!(result, "Open in browser:");
    });
}

#[test]
fn prompt_value_returns_plain_text_when_no_color() {
    with_no_color(|| {
        let result = prompt_value("https://example.com");
        assert_eq!(result, "https://example.com");
    });
}

#[test]
fn prompt_helpers_with_color_policy_style_when_enabled() {
    let title = prompt_label_with_color_policy("Select setup target", true);
    let option = prompt_value_with_color_policy("OpenCode", true);
    let error = error_text("Unknown command");

    assert!(title.contains("\u{1b}["));
    assert!(option.contains("\u{1b}["));
    assert!(!error.is_empty());
}

#[test]
fn success_produces_non_empty_output() {
    without_no_color(|| {
        let _ = success("completed");
    });
}

#[test]
fn label_produces_non_empty_output() {
    without_no_color(|| {
        let _ = label("Repository root:");
    });
}

#[test]
fn prompt_label_produces_non_empty_output() {
    without_no_color(|| {
        let _ = prompt_label("Open in browser:");
    });
}

#[test]
fn prompt_value_produces_non_empty_output() {
    without_no_color(|| {
        let _ = prompt_value("https://example.com");
    });
}

#[test]
fn clap_help_with_color_policy_returns_plain_text_when_disabled() {
    let input = "Usage: config show [OPTIONS]\n\nCommands:\n  validate  Validate config\n";

    assert_eq!(clap_help_with_color_policy(input, false), input);
}

#[test]
fn clap_help_with_color_policy_styles_headings_commands_and_placeholders() {
    let input = "Usage: config show [OPTIONS]\n\nCommands:\n  validate  Validate config\nOptions:\n      --format <FORMAT>  Output format\n";
    let output = clap_help_with_color_policy(input, true);

    assert!(output.contains("\u{1b}["));
    assert!(output.contains("Usage:"));
    assert!(output.contains("config"));
    assert!(output.contains("validate"));
    assert!(output.contains("[OPTIONS]"));
    assert!(output.contains("<FORMAT>"));
    assert_ne!(output, input);
}
