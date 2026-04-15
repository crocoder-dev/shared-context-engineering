# Plan: Colorful ASCII Banner with Right-to-Left Gradient on `sce help`

## Change summary

Add a colorful ASCII art banner (the "SCE" logo) to the top-level help output (`sce`, `sce help`, `sce --help`) with a per-column right-to-left color gradient. When color is disabled (piped output or `NO_COLOR`), the banner renders as plain ASCII without ANSI escapes. The gradient transitions from one color on the right side to another on the left, applied column-by-column across the banner width.

## Success criteria

- `sce`, `sce help`, and `sce --help` all display the ASCII art banner above the existing help sections.
- The banner uses a right-to-left color gradient (e.g., cyan on the right fading to magenta on the left) when stdout is a TTY and `NO_COLOR` is not set.
- The banner renders as plain uncolored ASCII when color is disabled (non-TTY or `NO_COLOR`).
- Existing help text content and layout below the banner is unchanged.
- `nix flake check` passes (build, clippy, fmt).
- No new dependencies are added; `owo-colors` v4 truecolor support (`.color(Rgb { r, g, b })`) is used for per-column gradient coloring.

## Constraints and non-goals

- **In scope**: Banner rendering in `command_surface::help_text()`, gradient logic in `style.rs`, `NO_COLOR`/TTY policy compliance, plain-text fallback.
- **Out of scope**: Changing any other command output, adding banners to subcommand help, adding a `--no-banner` flag, animated or blinking effects, changing the ASCII art shape from what the user provided.
- **Non-goal**: Supporting configurable gradient colors or banner content via CLI flags or config files in this change.

## Task stack

- [x] T01: `Add gradient banner rendering to style service` (status:done)
  - Task ID: T01
  - Goal: Add a `banner_with_gradient()` function (and a `_with_color_policy` variant) to `cli/src/services/style.rs` that takes the ASCII art lines and applies a per-column right-to-left RGB color gradient using `owo-colors` truecolor support, returning a styled String. When color is disabled, return the plain ASCII art unchanged.
  - Boundaries (in/out of scope): In — the gradient function, color interpolation logic, `_with_color_policy` variant for testability, `NO_COLOR`/TTY policy reuse via existing `supports_color()`. Out — integrating into `help_text()`, changing `command_surface.rs`, adding tests.
  - Done when: `banner_with_gradient()` and `banner_with_gradient_with_color_policy()` exist in `style.rs`, produce per-column colored output when color is enabled and plain output when disabled, and the crate compiles without warnings.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo build'` succeeds; manual inspection of the gradient function output for correct column-by-column coloring.

- [x] T02: `Integrate banner into help_text output` (status:done)
  - Task ID: T02
  - Goal: Modify `cli/src/command_surface.rs` `help_text()` to render the ASCII art banner at the top of the help output, before the "sce - Shared Context Engineering CLI" heading, using the gradient function from T01.
  - Boundaries (in/out of scope): In — adding the banner ASCII art constant, calling the gradient function, inserting the banner into `help_text()` output. Out — changing any other help section content or layout, modifying subcommand help.
  - Done when: `sce help` and `sce --help` display the colored banner at the top when run in a terminal; the banner is plain ASCII when piped or when `NO_COLOR=1`; all existing help sections appear unchanged below the banner.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo run -- help'` shows colored banner; `NO_COLOR=1 nix develop -c sh -c 'cd cli && cargo run -- help'` shows plain banner; `nix develop -c sh -c 'cd cli && cargo run -- help' | cat` shows plain banner.
  - **Status:** done
  - **Completed:** 2026-04-15
  - **Files changed:** `cli/src/command_surface.rs`
  - **Evidence:** `nix flake check` passes (build, clippy, fmt, tests); `sce help` and `sce --help` display the banner; `NO_COLOR=1` and piped output show plain ASCII banner; all existing help sections unchanged below banner.

- [~] T03: `Add unit tests for gradient banner rendering` (status:deferred)
  - Task ID: T03
  - Goal: Add unit tests in `cli/src/services/style.rs` (inline `#[cfg(test)]` module) for `banner_with_gradient_with_color_policy()` verifying: (1) color-enabled output contains ANSI escape sequences and the plain ASCII art text, (2) color-disabled output is identical to the plain ASCII art with no ANSI escapes, (3) the gradient function handles the banner width correctly (each column gets a distinct color).
  - Boundaries (in/out of scope): In — test module in `style.rs`, test cases for color-enabled and color-disabled paths, gradient column coverage. Out — integration tests, snapshot tests, changing production code.
  - Done when: Tests pass under `nix flake check` and `cargo test` covers both color policy paths.
  - Verification notes (commands or checks): `nix develop -c sh -c 'cd cli && cargo test style::tests'` passes; `nix flake check` passes.

- [x] T04: `Validation and context sync` (status:done)
  - Task ID: T04
  - Goal: Run full validation (`nix flake check`), verify the banner renders correctly in both color and plain modes, and update `context/cli/styling-service.md` and `context/cli/cli-command-surface.md` to document the banner and gradient function.
  - Boundaries (in/out of scope): In — `nix flake check`, manual banner verification, context file updates. Out — code changes beyond context files.
  - Done when: `nix flake check` passes; context files accurately reflect the new banner and gradient API.
  - Verification notes (commands or checks): `nix flake check`; review updated context files for accuracy.
  - **Status:** done
  - **Completed:** 2026-04-15
  - **Files changed:** `context/cli/styling-service.md`
  - **Evidence:** `nix flake check` passes; `nix run .#pkl-check-generated` passes; banner renders correctly in TTY (color), `NO_COLOR=1` (plain), and piped (plain) modes; `context/cli/styling-service.md` updated with spaces-left-unstyled behavioral note; `context/cli/cli-command-surface.md` was already accurate.

## Open questions

1. **Gradient color pair**: The plan assumes a cyan-to-magenta gradient (right=cyan, left=magenta) as a visually distinctive choice that complements the existing cyan headings and green command names. If you prefer different start/end colors, specify them before T01 implementation.
2. **Banner spacing**: The plan adds one blank line after the banner before the existing heading. If you prefer different vertical spacing, specify before T02.