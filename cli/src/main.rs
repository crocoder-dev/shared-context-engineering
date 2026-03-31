mod app;
mod cli_schema;
mod command_surface;
mod services;

use std::process::ExitCode;

fn main() -> ExitCode {
    app::run(std::env::args())
}

#[cfg(test)]
mod styling_audit {
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum AuditCategory {
        Help,
        StdoutText,
        StderrDiagnostic,
        PromptAdjacent,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum AuditStatus {
        Styled,
        MissingSharedStyling,
        ExcludedMachineReadable,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct AuditFinding {
        category: AuditCategory,
        location: &'static str,
        surface: &'static str,
        status: AuditStatus,
        notes: &'static str,
    }

    const HUMAN_READABLE_STYLE_AUDIT: &[AuditFinding] = &[
        AuditFinding {
            category: AuditCategory::Help,
            location: "cli/src/command_surface.rs::help_text",
            surface: "Top-level help headings, command names, and examples",
            status: AuditStatus::Styled,
            notes: "Uses shared heading() and command_name() helpers.",
        },
        AuditFinding {
            category: AuditCategory::Help,
            location: "cli/src/cli_schema.rs::render_help_for_path",
            surface: "Command-local clap long help payloads",
            status: AuditStatus::Styled,
            notes: "Post-processes clap help through shared heading/command/placeholder styling helpers.",
        },
        AuditFinding {
            category: AuditCategory::Help,
            location: "cli/src/cli_schema.rs::auth_help_text",
            surface: "Bare auth help combines styled examples with raw clap base help",
            status: AuditStatus::Styled,
            notes: "Inherited auth help body is styled via shared clap-help rendering and examples stay styled.",
        },
        AuditFinding {
            category: AuditCategory::StdoutText,
            location: "cli/src/services/auth_command.rs::render_login_result",
            surface: "Auth login text success payload",
            status: AuditStatus::Styled,
            notes: "Uses success(), prompt_label(), prompt_value(), label(), and value().",
        },
        AuditFinding {
            category: AuditCategory::StdoutText,
            location: "cli/src/services/config.rs::render_*_text",
            surface: "Config text reports",
            status: AuditStatus::Styled,
            notes: "Uses shared success()/label()/value() helpers across text renderers.",
        },
        AuditFinding {
            category: AuditCategory::StdoutText,
            location: "cli/src/services/doctor.rs::format_execution",
            surface: "Doctor text report",
            status: AuditStatus::Styled,
            notes: "Uses shared heading()/success()/label()/value() helpers.",
        },
        AuditFinding {
            category: AuditCategory::StdoutText,
            location: "cli/src/services/setup.rs::render_*",
            surface: "Setup text outcomes",
            status: AuditStatus::Styled,
            notes: "Uses shared success()/label()/value() helpers for human-readable output.",
        },
        AuditFinding {
            category: AuditCategory::StdoutText,
            location: "cli/src/services/sync.rs::run_placeholder_sync",
            surface: "Sync placeholder text report",
            status: AuditStatus::Styled,
            notes: "Uses shared label()/command_name()/value() helpers.",
        },
        AuditFinding {
            category: AuditCategory::StdoutText,
            location: "cli/src/services/trace.rs::render_prompt_trace_text",
            surface: "Trace prompts text report",
            status: AuditStatus::Styled,
            notes: "Uses shared label()/value() helpers for headers and summary fields.",
        },
        AuditFinding {
            category: AuditCategory::StdoutText,
            location: "cli/src/services/version.rs::render_version",
            surface: "Version text report",
            status: AuditStatus::Styled,
            notes: "Uses shared command_name() and value() helpers.",
        },
        AuditFinding {
            category: AuditCategory::StderrDiagnostic,
            location: "cli/src/app.rs::write_error_diagnostic",
            surface: "Top-level human-readable stderr diagnostics",
            status: AuditStatus::MissingSharedStyling,
            notes: "Only the error code uses styling; the diagnostic body is otherwise raw text.",
        },
        AuditFinding {
            category: AuditCategory::StderrDiagnostic,
            location: "cli/src/services/observability.rs::Logger::log",
            surface: "Fallback stderr message when log-file mirroring fails",
            status: AuditStatus::MissingSharedStyling,
            notes: "Writes a raw human-readable stderr message via eprintln!().",
        },
        AuditFinding {
            category: AuditCategory::PromptAdjacent,
            location: "cli/src/services/auth_command.rs::write_login_prompt",
            surface: "Auth device-flow prompt guidance",
            status: AuditStatus::Styled,
            notes: "Uses prompt_label(), prompt_value(), and value().",
        },
        AuditFinding {
            category: AuditCategory::PromptAdjacent,
            location: "cli/src/services/setup.rs::InquireSetupTargetPrompter::prompt_target",
            surface: "Interactive setup prompt title",
            status: AuditStatus::MissingSharedStyling,
            notes: "Prompt title is a raw string passed to Select::new().",
        },
        AuditFinding {
            category: AuditCategory::PromptAdjacent,
            location: "cli/src/services/setup.rs::SetupPromptTarget::fmt",
            surface: "Interactive setup prompt choice labels",
            status: AuditStatus::MissingSharedStyling,
            notes: "Choice labels render as raw Display strings without shared prompt styling.",
        },
        AuditFinding {
            category: AuditCategory::Help,
            location: "cli/src/services/completion.rs",
            surface: "Completion script output",
            status: AuditStatus::ExcludedMachineReadable,
            notes: "Completion output is machine-readable and intentionally excluded.",
        },
        AuditFinding {
            category: AuditCategory::StdoutText,
            location: "cli/src/services/*::Json renderers",
            surface: "JSON output payloads",
            status: AuditStatus::ExcludedMachineReadable,
            notes: "JSON output is machine-readable and intentionally excluded.",
        },
    ];

    fn has_finding(location: &str, status: AuditStatus) -> bool {
        HUMAN_READABLE_STYLE_AUDIT
            .iter()
            .any(|finding| finding.location == location && finding.status == status)
    }

    #[test]
    fn audit_covers_all_in_scope_human_readable_surface_categories() {
        for category in [
            AuditCategory::Help,
            AuditCategory::StdoutText,
            AuditCategory::StderrDiagnostic,
            AuditCategory::PromptAdjacent,
        ] {
            assert!(
                HUMAN_READABLE_STYLE_AUDIT
                    .iter()
                    .any(|finding| finding.category == category),
                "missing audit coverage for {category:?}"
            );
        }
    }

    #[test]
    fn audit_records_current_missing_shared_styling_paths() {
        for location in [
            "cli/src/app.rs::write_error_diagnostic",
            "cli/src/services/observability.rs::Logger::log",
            "cli/src/services/setup.rs::InquireSetupTargetPrompter::prompt_target",
            "cli/src/services/setup.rs::SetupPromptTarget::fmt",
        ] {
            assert!(
                has_finding(location, AuditStatus::MissingSharedStyling),
                "expected missing-style audit finding for {location}"
            );
        }
    }

    #[test]
    fn audit_keeps_machine_readable_surfaces_out_of_scope() {
        for location in [
            "cli/src/services/completion.rs",
            "cli/src/services/*::Json renderers",
        ] {
            assert!(
                has_finding(location, AuditStatus::ExcludedMachineReadable),
                "expected machine-readable exclusion for {location}"
            );
        }
    }

    #[test]
    fn audit_marks_t02_help_surfaces_as_styled() {
        for location in [
            "cli/src/cli_schema.rs::render_help_for_path",
            "cli/src/cli_schema.rs::auth_help_text",
        ] {
            assert!(
                has_finding(location, AuditStatus::Styled),
                "expected styled audit finding for {location}"
            );
        }
    }
}
