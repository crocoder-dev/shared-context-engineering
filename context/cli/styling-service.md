# CLI Styling Service

The CLI styling service in `cli/src/services/style.rs` provides deterministic text-mode output styling for human-facing CLI surfaces.

## Dependencies

- `owo-colors` - Color styling with automatic TTY detection

## API

### Color Support Detection

- `supports_color() -> bool` - Returns `true` if stdout is a TTY and `NO_COLOR` is not set
- `supports_color_stderr() -> bool` - Returns `true` if stderr is a TTY and `NO_COLOR` is not set

### Conditional Styling

- `style_if_enabled<F>(text: &str, f: F) -> String` - Applies styling function only when colors are enabled
- `success_with_stderr_color_policy(text: &str, color_enabled: bool) -> String` - Internal helper for applying the shared green/bold stderr success policy when a caller already resolved the color decision

### Help Output Styling

- `heading(text: &str) -> String` - Styles section headings (cyan/bold) for help output
- `command_name(text: &str) -> String` - Styles command names (green) for help output
- `clap_help(text: &str) -> String` - Post-processes command-local clap help text so stdout help surfaces reuse shared heading, command, and placeholder styling without changing plain-text output when color is disabled

### Internal Error Diagnostics Styling

- `error_code(text: &str) -> String` - Styles error codes (red/bold) for internal stderr diagnostics
- `error_code_with_color_policy(text: &str, color_enabled: bool) -> String` - Internal variant accepting an explicit color policy flag for testability
- `heading(text: &str) -> String` - Styles headings for both stdout and internal stderr output (cyan/bold)
- `error_text_with_color_policy(text: &str, color_enabled: bool) -> String` - Internal helper styling human-readable internal stderr diagnostic bodies (yellow) given an explicit color policy flag; `app_support::write_error_diagnostic` is the sole production caller, passing `supports_color_stderr()`
- Catalog messages for expected failures are intentionally emitted redacted but unstyled and without the internal diagnostic wrapper.

### Command Output Styling

- `success(text: &str) -> String` - Styles success states/labels (green/bold) for command output
- `label(text: &str) -> String` - Styles field labels (cyan) for key-value output
- `value(text: &str) -> String` - Returns values unchanged (for consistency with future styling)
- `prompt_label(text: &str) -> String` - Styles prompt labels (bold) for interactive prompts
- `prompt_value(text: &str) -> String` - Styles prompt values (yellow) for user-actionable items like URLs and codes
- Interactive `sce setup` prompt titles and target-choice labels now reuse those same prompt helpers instead of raw strings.

### Banner Gradient Styling

- `banner_with_gradient(lines: &[&str]) -> String` - Applies a per-column right-to-left color gradient (cyan on the right, magenta on the left) to ASCII art banner lines when color is enabled; returns plain ASCII when color is disabled; spaces in the banner are left unstyled to avoid trailing-space ANSI artifacts
- `banner_with_gradient_with_color_policy(lines: &[&str], color_enabled: bool) -> String` - Internal variant accepting an explicit color policy flag for testability

### Policy

- `NO_COLOR` environment variable is respected per no-color.org specification
- Non-TTY output (piped/redirected) automatically disables colors
- JSON output paths remain unstyled
- Completion scripts and MCP stdio outputs remain unstyled
- Help output uses `supports_color()` for stdout TTY detection
- Command-local help styling is applied after clap renders plain help text, covering `Usage:`, section headings, command rows, and placeholder tokens on stdout surfaces
- Error diagnostics use `supports_color_stderr()` for stderr TTY detection
- Top-level internal app diagnostics and observability log-file write failures render through the shared stderr styling helpers when stderr color is enabled; user catalog diagnostics intentionally bypass those helpers.

## Sync progress styling

The sync-owned `cli/src/services/sync/progress.rs` presentation consumer keeps human progress on
`stderr` and uses `supports_color_stderr()` plus `NO_COLOR` for its completion
marker. The presentation adapter owns the `indicatif` multi-progress rows;
this service owns the shared green/bold completion-marker policy through
`success_with_stderr_color_policy(...)`. Spinner rows use plain text when
stderr is redirected or color is disabled; non-TTY snapshots never emit ANSI
or terminal-control sequences. JSON output remains silent on the
human-progress channel.
The generic reporter contract and its no-op implementation remain owned by the
sync progress module; this styling service owns only the shared color policy.

## Re-exports

- `pub use owo_colors::OwoColorize` - Trait for color styling methods on strings

## Usage

```rust
use crate::services::style::{heading, command_name, error_code, error_text_with_color_policy, success, label, value, prompt_label, prompt_value, supports_color, supports_color_stderr};

// Help output styling
println!("{}", heading("Usage:"));
println!("  {}", command_name("sce setup"));

// Internal error diagnostics styling (stderr)
eprintln!(
    "{} [{}]: {}",
    heading("Error"),
    error_code("SCE-ERR-PARSE"),
    error_text_with_color_policy(message, supports_color_stderr())
);

// Catalog messages are redacted and written without styling or wrapper.

// Command output styling
println!("{}", success("Setup completed successfully."));
println!("{} {}", label("Repository root:"), value("'/path/to/repo'"));

// Interactive prompt styling
println!("{} {}", prompt_label("Open in browser:"), prompt_value("https://example.com"));
println!("{} {}", prompt_label("Code:"), prompt_value("ABCD-EFGH"));

// Conditional styling
if supports_color() {
    println!("{}", "Success".green());
}
```

## See also

- [overview.md](../overview.md)
- [context-map.md](../context-map.md)
