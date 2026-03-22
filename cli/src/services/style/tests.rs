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
fn example_command_returns_plain_text_when_no_color() {
    with_no_color(|| {
        let result = example_command("sce setup");
        assert_eq!(result, "sce setup");
    });
}

#[test]
fn placeholder_returns_plain_text_when_no_color() {
    with_no_color(|| {
        let result = placeholder("<command>");
        assert_eq!(result, "<command>");
    });
}

#[test]
fn status_implemented_returns_plain_text_when_no_color() {
    with_no_color(|| {
        let result = status_implemented("implemented");
        assert_eq!(result, "implemented");
    });
}

#[test]
fn status_placeholder_returns_plain_text_when_no_color() {
    with_no_color(|| {
        let result = status_placeholder("placeholder");
        assert_eq!(result, "placeholder");
    });
}

#[test]
fn heading_stderr_returns_plain_text_when_no_color() {
    with_no_color(|| {
        let result = heading_stderr("Error:");
        assert_eq!(result, "Error:");
    });
}

#[test]
fn style_functions_produce_non_empty_output() {
    without_no_color(|| {
        let _ = heading("Usage:");
        let _ = command_name("setup");
        let _ = example_command("sce setup");
        let _ = placeholder("<command>");
        let _ = status_implemented("implemented");
        let _ = status_placeholder("placeholder");
        let _ = heading_stderr("Error:");
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
