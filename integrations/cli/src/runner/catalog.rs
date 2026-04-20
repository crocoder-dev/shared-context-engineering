pub(super) const COMPLETION_BASH_REQUIRED_MARKERS: &[&str] = &["_sce()", "complete -F _sce"];
pub(super) const COMPLETION_ZSH_REQUIRED_MARKERS: &[&str] = &["#compdef sce", "_arguments"];
pub(super) const COMPLETION_FISH_REQUIRED_MARKERS: &[&str] = &["complete -c sce"];
pub(super) const CONFIG_PRECEDENCE_TEXT: &str = "flags > env > config file > defaults";

#[derive(Clone, Copy)]
pub(super) struct CommandSuite {
    pub(super) name: &'static str,
    pub(super) cases: &'static [CommandCase],
}

#[derive(Clone, Copy)]
pub(super) enum ExpectedStatus {
    Success,
}

#[derive(Clone, Copy)]
pub(super) struct OutputExpectation {
    pub(super) must_be_empty: bool,
    pub(super) must_be_non_empty: bool,
    pub(super) required_substrings: &'static [&'static str],
    pub(super) validator: Option<OutputValidator>,
}

pub(super) type OutputValidator = fn(&str) -> Result<(), String>;

impl OutputExpectation {
    pub(super) const fn non_empty() -> Self {
        Self {
            must_be_empty: false,
            must_be_non_empty: true,
            required_substrings: &[],
            validator: None,
        }
    }

    pub(super) const fn with_required_substrings(
        mut self,
        required_substrings: &'static [&'static str],
    ) -> Self {
        self.required_substrings = required_substrings;
        self
    }

    pub(super) const fn with_validator(mut self, validator: OutputValidator) -> Self {
        self.validator = Some(validator);
        self
    }
}

#[derive(Clone, Copy)]
pub(super) struct CaseExpectation {
    pub(super) status: ExpectedStatus,
    pub(super) stdout: OutputExpectation,
}

#[derive(Clone, Copy)]
pub(super) struct CommandCase {
    pub(super) name: &'static str,
    pub(super) argv: &'static [&'static str],
    pub(super) expectation: CaseExpectation,
}

const HELP_CASES: &[CommandCase] = &[CommandCase {
    name: "top-level-help",
    argv: &["--help"],
    expectation: CaseExpectation {
        status: ExpectedStatus::Success,
        stdout: OutputExpectation::non_empty().with_required_substrings(&["Usage:"]),
    },
}];

const VERSION_CASES: &[CommandCase] = &[
    CommandCase {
        name: "version-default-text",
        argv: &["version"],
        expectation: CaseExpectation {
            status: ExpectedStatus::Success,
            stdout: OutputExpectation::non_empty()
                .with_validator(super::validators::validate_version_text_output),
        },
    },
    CommandCase {
        name: "version-explicit-text-format",
        argv: &["version", "--format", "text"],
        expectation: CaseExpectation {
            status: ExpectedStatus::Success,
            stdout: OutputExpectation::non_empty()
                .with_validator(super::validators::validate_version_text_output),
        },
    },
    CommandCase {
        name: "version-json-format",
        argv: &["version", "--format", "json"],
        expectation: CaseExpectation {
            status: ExpectedStatus::Success,
            stdout: OutputExpectation::non_empty()
                .with_validator(super::validators::validate_version_json_output),
        },
    },
    CommandCase {
        name: "top-level-version-long-flag",
        argv: &["--version"],
        expectation: CaseExpectation {
            status: ExpectedStatus::Success,
            stdout: OutputExpectation::non_empty()
                .with_validator(super::validators::validate_version_text_output),
        },
    },
    CommandCase {
        name: "top-level-version-short-flag",
        argv: &["-V"],
        expectation: CaseExpectation {
            status: ExpectedStatus::Success,
            stdout: OutputExpectation::non_empty()
                .with_validator(super::validators::validate_version_text_output),
        },
    },
];

const COMPLETION_CASES: &[CommandCase] = &[
    CommandCase {
        name: "completion-bash",
        argv: &["completion", "--shell", "bash"],
        expectation: CaseExpectation {
            status: ExpectedStatus::Success,
            stdout: OutputExpectation::non_empty()
                .with_validator(super::validators::validate_completion_bash_output),
        },
    },
    CommandCase {
        name: "completion-zsh",
        argv: &["completion", "--shell", "zsh"],
        expectation: CaseExpectation {
            status: ExpectedStatus::Success,
            stdout: OutputExpectation::non_empty()
                .with_validator(super::validators::validate_completion_zsh_output),
        },
    },
    CommandCase {
        name: "completion-fish",
        argv: &["completion", "--shell", "fish"],
        expectation: CaseExpectation {
            status: ExpectedStatus::Success,
            stdout: OutputExpectation::non_empty()
                .with_validator(super::validators::validate_completion_fish_output),
        },
    },
];

const CONFIG_CASES: &[CommandCase] = &[
    CommandCase {
        name: "config-show-text-format",
        argv: &["config", "show", "--format", "text"],
        expectation: CaseExpectation {
            status: ExpectedStatus::Success,
            stdout: OutputExpectation::non_empty()
                .with_validator(super::validators::validate_config_show_text_output),
        },
    },
    CommandCase {
        name: "config-show-json-format",
        argv: &["config", "show", "--format", "json"],
        expectation: CaseExpectation {
            status: ExpectedStatus::Success,
            stdout: OutputExpectation::non_empty()
                .with_validator(super::validators::validate_config_show_json_output),
        },
    },
    CommandCase {
        name: "config-validate-text-format",
        argv: &["config", "validate", "--format", "text"],
        expectation: CaseExpectation {
            status: ExpectedStatus::Success,
            stdout: OutputExpectation::non_empty()
                .with_validator(super::validators::validate_config_validate_text_output),
        },
    },
    CommandCase {
        name: "config-validate-json-format",
        argv: &["config", "validate", "--format", "json"],
        expectation: CaseExpectation {
            status: ExpectedStatus::Success,
            stdout: OutputExpectation::non_empty()
                .with_validator(super::validators::validate_config_validate_json_output),
        },
    },
];

const DOCTOR_CASES: &[CommandCase] = &[
    CommandCase {
        name: "doctor-text-format",
        argv: &["doctor", "--format", "text"],
        expectation: CaseExpectation {
            status: ExpectedStatus::Success,
            stdout: OutputExpectation::non_empty()
                .with_validator(super::validators::validate_doctor_text_output),
        },
    },
    CommandCase {
        name: "doctor-json-format",
        argv: &["doctor", "--format", "json"],
        expectation: CaseExpectation {
            status: ExpectedStatus::Success,
            stdout: OutputExpectation::non_empty()
                .with_validator(super::validators::validate_doctor_json_output),
        },
    },
];

pub(super) const COMMAND_SUITES: &[CommandSuite] = &[
    CommandSuite {
        name: "help",
        cases: HELP_CASES,
    },
    CommandSuite {
        name: "version",
        cases: VERSION_CASES,
    },
    CommandSuite {
        name: "completion",
        cases: COMPLETION_CASES,
    },
    CommandSuite {
        name: "config",
        cases: CONFIG_CASES,
    },
    CommandSuite {
        name: "doctor",
        cases: DOCTOR_CASES,
    },
];
