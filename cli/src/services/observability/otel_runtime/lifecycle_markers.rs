#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LifecycleMarker {
    CommandComplete,
    ShutdownBegin,
    ShutdownEnd,
    ProcessExitRequested,
}

impl LifecycleMarker {
    #[cfg(feature = "telemetry-test-receiver")]
    fn as_str(self) -> &'static str {
        match self {
            Self::CommandComplete => "command_complete",
            Self::ShutdownBegin => "shutdown_begin",
            Self::ShutdownEnd => "shutdown_end",
            Self::ProcessExitRequested => "process_exit_requested",
        }
    }
}

#[cfg(feature = "telemetry-test-receiver")]
pub const LIFECYCLE_FILE_ENV: &str = "SCE_TELEMETRY_TEST_LIFECYCLE_FILE";

#[cfg(feature = "telemetry-test-receiver")]
pub fn mark(marker: LifecycleMarker) {
    use std::io::Write;
    use std::sync::OnceLock;
    use std::time::Instant;

    static ORIGIN: OnceLock<Instant> = OnceLock::new();

    let Some(path) = std::env::var_os(LIFECYCLE_FILE_ENV) else {
        return;
    };
    let elapsed = ORIGIN.get_or_init(Instant::now).elapsed();
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    else {
        return;
    };
    let _ = writeln!(file, "{} {}", marker.as_str(), elapsed.as_nanos());
}

#[cfg(not(feature = "telemetry-test-receiver"))]
pub fn mark(_marker: LifecycleMarker) {}
