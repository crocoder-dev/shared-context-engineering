# CLI Styling Service

The CLI styling service in `cli/src/services/style.rs` provides deterministic text-mode output styling for human-facing CLI surfaces.

## Dependencies

- `owo-colors` - Color styling with automatic TTY detection
- `comfy-table` - Table rendering for tabular output

## API

### Color Support Detection

- `supports_color() -> bool` - Returns `true` if stdout is a TTY and `NO_COLOR` is not set
- `supports_color_stderr() -> bool` - Returns `true` if stderr is a TTY and `NO_COLOR` is not set

### Table Rendering

- `table() -> Table` - Creates a new `comfy_table::Table` instance for tabular output
- `create_table(headers: &[&str]) -> Table` - Creates a styled table with compact preset (no borders), applies cyan/bold header styling when color is enabled, and returns a table ready for row additions

### Conditional Styling

- `style_if_enabled<F>(text: &str, f: F) -> String` - Applies styling function only when colors are enabled
- `style_if_enabled_stderr<F>(text: &str, f: F) -> String` - Applies styling function only when stderr colors are enabled

### Help Output Styling

- `heading(text: &str) -> String` - Styles section headings (cyan/bold) for help output
- `command_name(text: &str) -> String` - Styles command names (green) for help output
- `example_command(text: &str) -> String` - Styles usage examples (yellow)
- `placeholder(text: &str) -> String` - Styles placeholders/arguments (dim/italic)
- `clap_help(text: &str) -> String` - Post-processes command-local clap help text so stdout help surfaces reuse shared heading, command, and placeholder styling without changing plain-text output when color is disabled
- `status_implemented(text: &str) -> String` - Styles "implemented" status (green)
- `status_placeholder(text: &str) -> String` - Styles "placeholder" status (dimmed)

### Error Diagnostics Styling

- `error_code(text: &str) -> String` - Styles error codes (red/bold) for stderr diagnostics
- `heading_stderr(text: &str) -> String` - Styles headings for stderr output (cyan/bold)
- `error_text(text: &str) -> String` - Styles human-readable stderr diagnostic bodies (yellow)

### Command Output Styling

- `success(text: &str) -> String` - Styles success states/labels (green/bold) for command output
- `label(text: &str) -> String` - Styles field labels (cyan) for key-value output
- `value(text: &str) -> String` - Returns values unchanged (for consistency with future styling)
- `prompt_label(text: &str) -> String` - Styles prompt labels (bold) for interactive prompts
- `prompt_value(text: &str) -> String` - Styles prompt values (yellow) for user-actionable items like URLs and codes
- Interactive `sce setup` prompt titles and target-choice labels now reuse those same prompt helpers instead of raw strings.

## Policy

- `NO_COLOR` environment variable is respected per no-color.org specification
- Non-TTY output (piped/redirected) automatically disables colors
- JSON output paths remain unstyled
- Completion scripts and MCP stdio outputs remain unstyled
- Help output uses `supports_color()` for stdout TTY detection
- Command-local help styling is applied after clap renders plain help text, covering `Usage:`, section headings, command rows, and placeholder tokens on stdout surfaces
- Error diagnostics use `supports_color_stderr()` for stderr TTY detection
- Top-level app diagnostics and observability log-file write failures both render through the shared stderr styling helpers when stderr color is enabled.

## Re-exports

- `pub use owo_colors::OwoColorize` - Trait for color styling methods on strings
- `pub use comfy_table::Table` - Table type for tabular output

## Usage

```rust
use crate::services::style::{heading, command_name, error_code, error_text, success, label, value, prompt_label, prompt_value, create_table, supports_color};

// Help output styling
println!("{}", heading("Usage:"));
println!("  {}", command_name("sce setup"));

// Error diagnostics styling (stderr)
eprintln!("{} [{}]: {}", heading_stderr("Error"), error_code("SCE-ERR-PARSE"), error_text(message));

// Command output styling
println!("{}", success("Setup completed successfully."));
println!("{} {}", label("Repository root:"), value("'/path/to/repo'"));

// Interactive prompt styling
println!("{} {}", prompt_label("Open in browser:"), prompt_value("https://example.com"));
println!("{} {}", prompt_label("Code:"), prompt_value("ABCD-EFGH"));

// Table output styling
let mut table = create_table(&["Command", "Status", "Purpose"]);
table.add_row(vec!["setup", "implemented", "Prepare local repository prerequisites"]);
table.add_row(vec!["sync", "placeholder", "Coordinate future cloud sync workflows"]);
println!("{}", table);

// Conditional styling
if supports_color() {
    println!("{}", "Success".green());
}
```

## See also

- [overview.md](../overview.md)
- [context-map.md](../context-map.md)
