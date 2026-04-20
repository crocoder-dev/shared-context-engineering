mod completion;
mod config;
mod doctor;
mod shared;
mod version;

pub(super) fn validate_completion_bash_output(stream: &str) -> Result<(), String> {
    completion::validate_completion_bash_output(stream)
}

pub(super) fn validate_completion_zsh_output(stream: &str) -> Result<(), String> {
    completion::validate_completion_zsh_output(stream)
}

pub(super) fn validate_completion_fish_output(stream: &str) -> Result<(), String> {
    completion::validate_completion_fish_output(stream)
}

pub(super) fn validate_config_show_text_output(stream: &str) -> Result<(), String> {
    config::validate_config_show_text_output(stream)
}

pub(super) fn validate_config_show_json_output(stream: &str) -> Result<(), String> {
    config::validate_config_show_json_output(stream)
}

pub(super) fn validate_config_validate_text_output(stream: &str) -> Result<(), String> {
    config::validate_config_validate_text_output(stream)
}

pub(super) fn validate_config_validate_json_output(stream: &str) -> Result<(), String> {
    config::validate_config_validate_json_output(stream)
}

pub(super) fn validate_doctor_text_output(stream: &str) -> Result<(), String> {
    doctor::validate_doctor_text_output(stream)
}

pub(super) fn validate_doctor_json_output(stream: &str) -> Result<(), String> {
    doctor::validate_doctor_json_output(stream)
}

pub(super) fn validate_version_text_output(stream: &str) -> Result<(), String> {
    version::validate_version_text_output(stream)
}

pub(super) fn validate_version_json_output(stream: &str) -> Result<(), String> {
    version::validate_version_json_output(stream)
}
