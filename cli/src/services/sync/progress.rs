//! Sync-owned progress reporting contracts and terminal presentation.

use std::io::{self, IsTerminal, Write};
use std::time::Duration;

use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle};

use super::sync::SyncProgressEvent;

/// Receives deterministic progress events and can finalize a successful run.
pub trait ProgressReporter<E> {
    fn report(&mut self, event: E);

    /// Finalizes a successfully completed report.
    ///
    /// Reporters that do not need a separate successful-finalization step can
    /// keep the default no-op implementation.
    fn finish_successfully(&mut self) {}
}

impl<F, E> ProgressReporter<E> for F
where
    F: FnMut(E),
{
    fn report(&mut self, event: E) {
        self(event);
    }
}

/// Discards progress for callers that only need the final result.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoopProgressReporter;

impl<E> ProgressReporter<E> for NoopProgressReporter {
    fn report(&mut self, _event: E) {}
}

const STREAM_LABELS: [&str; 3] = ["messages", "parts", "agent_traces"];
const STREAM_LABEL_WIDTH: usize = 15;
const STEADY_TICK_INTERVAL: Duration = Duration::from_millis(100);

pub struct IndicatifProgressReporter<'a, W> {
    _progress: MultiProgress,
    bars: [ProgressBar; STREAM_LABELS.len()],
    writer: &'a mut W,
    interactive: bool,
    color_enabled: bool,
    uploaded: [usize; STREAM_LABELS.len()],
    completed: [bool; STREAM_LABELS.len()],
}

impl<'a, W> IndicatifProgressReporter<'a, W>
where
    W: Write,
{
    pub fn new(writer: &'a mut W) -> Self {
        Self::with_policies(
            writer,
            io::stderr().is_terminal(),
            crate::services::style::supports_color_stderr(),
        )
    }

    pub(crate) fn with_policies(writer: &'a mut W, interactive: bool, color_enabled: bool) -> Self {
        let target = if interactive {
            ProgressDrawTarget::stderr_with_hz(20)
        } else {
            ProgressDrawTarget::hidden()
        };
        let progress = MultiProgress::with_draw_target(target);
        let style = progress_style(color_enabled);
        let bars = STREAM_LABELS.map(|stream| {
            let bar = progress.add(ProgressBar::new(0));
            bar.set_style(style.clone());
            bar.set_prefix(format!("{stream:<STREAM_LABEL_WIDTH$}"));
            bar.set_message(uploaded_message(0));
            bar.enable_steady_tick(STEADY_TICK_INTERVAL);
            bar
        });

        let mut reporter = Self {
            _progress: progress,
            bars,
            writer,
            interactive,
            color_enabled,
            uploaded: [0; STREAM_LABELS.len()],
            completed: [false; STREAM_LABELS.len()],
        };
        reporter.render_non_interactive();
        reporter
    }

    fn bar_for(&self, stream: &str) -> Option<&ProgressBar> {
        STREAM_LABELS
            .iter()
            .position(|candidate| *candidate == stream)
            .map(|index| &self.bars[index])
    }

    fn stream_index(stream: &str) -> Option<usize> {
        STREAM_LABELS
            .iter()
            .position(|candidate| *candidate == stream)
    }

    fn report_batch(&mut self, stream: &str, uploaded: usize) {
        if let Some(bar) = self.bar_for(stream) {
            bar.set_position(uploaded as u64);
            bar.set_message(uploaded_message(uploaded));
        }
        if let Some(index) = Self::stream_index(stream) {
            self.uploaded[index] = uploaded;
        }
        self.render_non_interactive();
    }

    fn report_completion(&mut self, stream: &str, uploaded: usize) {
        if let Some(bar) = self.bar_for(stream) {
            bar.finish_with_message(uploaded_message(uploaded));
        }
        if let Some(index) = Self::stream_index(stream) {
            self.uploaded[index] = uploaded;
            self.completed[index] = true;
        }
        self.render_non_interactive();
    }

    fn render_non_interactive(&mut self) {
        if self.interactive {
            return;
        }

        for (index, stream) in STREAM_LABELS.iter().enumerate() {
            let marker = if self.completed[index] {
                crate::services::style::success_with_stderr_color_policy("✓", self.color_enabled)
            } else {
                "⠋".to_string()
            };
            let _ = writeln!(
                self.writer,
                "{marker} {stream:<STREAM_LABEL_WIDTH$} {}",
                uploaded_message(self.uploaded[index])
            );
        }
        let _ = self.writer.flush();
    }
}

impl<W> ProgressReporter<SyncProgressEvent> for IndicatifProgressReporter<'_, W>
where
    W: Write,
{
    fn report(&mut self, event: SyncProgressEvent) {
        match event {
            SyncProgressEvent::Started { .. } | SyncProgressEvent::Finished { .. } => {
                self.render_non_interactive();
            }
            SyncProgressEvent::BatchAccepted {
                stream, uploaded, ..
            } => self.report_batch(stream, uploaded),
            SyncProgressEvent::StreamCompleted {
                stream, uploaded, ..
            } => self.report_completion(stream, uploaded),
        }
    }

    fn finish_successfully(&mut self) {
        let line_breaks = if self.interactive { 2 } else { 1 };
        for _ in 0..line_breaks {
            let _ = writeln!(self.writer);
        }
        let _ = self.writer.flush();
    }
}

fn progress_style(color_enabled: bool) -> ProgressStyle {
    let check = crate::services::style::success_with_stderr_color_policy("✓", color_enabled);
    ProgressStyle::with_template("{spinner} {prefix} {msg}")
        .expect("sync progress template is valid")
        .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", check.as_str()])
}

fn uploaded_message(uploaded: usize) -> String {
    let row_label = if uploaded == 1 { "row" } else { "rows" };
    format!("{uploaded} {row_label} uploaded")
}
