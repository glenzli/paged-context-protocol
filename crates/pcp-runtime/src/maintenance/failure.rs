//! Stable maintenance diagnostics. Unknown provider usage is never settled by classification.
use std::{fmt, time::Duration};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FailureKind {
    InferenceTimeout,
    ResponseTimeout,
    ExecutionInterrupted,
    InvalidOutput,
    WorkerFailed,
}

impl FailureKind {
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::InferenceTimeout => "inference_timeout",
            Self::ResponseTimeout => "response_timeout",
            Self::ExecutionInterrupted => "execution_interrupted",
            Self::InvalidOutput => "invalid_model_output",
            Self::WorkerFailed => "worker_failed",
        }
    }

    pub(crate) fn from_message(message: &str) -> Self {
        if message.contains("maintenance wait interrupted") {
            Self::ExecutionInterrupted
        } else if message.contains("maintenance response wait timed out")
            || message.contains("maintenance submission timed out")
            || message.contains("maintenance response timed out")
        {
            Self::ResponseTimeout
        } else if message.contains("deadline_exceeded")
            || message.contains("maintenance inference timed out")
        {
            Self::InferenceTimeout
        } else if message.contains("maintenance output invalid")
            || message.contains("decode strict PCP maintenance decision")
        {
            Self::InvalidOutput
        } else {
            Self::WorkerFailed
        }
    }

    pub(crate) fn from_error(error: &anyhow::Error) -> Self {
        error
            .downcast_ref::<WorkerFailure>()
            .map(|e| e.kind)
            .unwrap_or_else(|| Self::from_message(&format!("{error:#}")))
    }
}

#[derive(Debug)]
pub(crate) struct WorkerFailure {
    pub(crate) kind: FailureKind,
    message: String,
}
impl fmt::Display for WorkerFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for WorkerFailure {}
impl WorkerFailure {
    pub(crate) fn invalid_output() -> Self {
        Self {
            kind: FailureKind::InvalidOutput,
            message: "Infer Runtime maintenance output invalid; original evidence retained".into(),
        }
    }

    pub(crate) fn wait(
        kind: FailureKind,
        budget: Duration,
        active: Duration,
        wall: Duration,
    ) -> Self {
        // Monotonic time can stop during macOS sleep. A wall-clock jump is evidence
        // of an interruption, not proof of sleep (clock adjustments can do this too).
        if wall > active.saturating_add(Duration::from_secs(15)) {
            return Self {
                kind: FailureKind::ExecutionInterrupted,
                message: format!(
                    "Infer Runtime maintenance wait interrupted: possible system sleep or clock change (active {}s, wall {}s); original evidence retained",
                    active.as_secs(),
                    wall.as_secs()
                ),
            };
        }
        let message = if kind == FailureKind::InferenceTimeout {
            format!(
                "Infer Runtime maintenance inference timed out after {}s; original evidence retained",
                budget.as_secs()
            )
        } else {
            format!(
                "Infer Runtime maintenance response wait timed out after {}s (inference deadline {}s); execution outcome unknown; original evidence retained",
                active.as_secs(),
                budget.as_secs()
            )
        };
        Self { kind, message }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn timeout_and_interruption_preserve_distinct_meanings() {
        let seconds = Duration::from_secs;
        let response = WorkerFailure::wait(
            FailureKind::ResponseTimeout,
            seconds(120),
            seconds(130),
            seconds(130),
        );
        assert_eq!(response.kind, FailureKind::ResponseTimeout);
        assert!(response.to_string().contains("outcome unknown"));
        let suspended = WorkerFailure::wait(
            FailureKind::InferenceTimeout,
            seconds(120),
            seconds(120),
            seconds(3600),
        );
        assert_eq!(suspended.kind, FailureKind::ExecutionInterrupted);
        assert_eq!(
            FailureKind::from_message(&suspended.to_string()),
            suspended.kind
        );
        let adjusted_backwards = WorkerFailure::wait(
            FailureKind::InferenceTimeout,
            seconds(120),
            seconds(120),
            seconds(60),
        );
        assert_eq!(adjusted_backwards.kind, FailureKind::InferenceTimeout);
    }
    #[test]
    fn context_keeps_typed_diagnostic_and_legacy_errors_remain_readable() {
        let error =
            anyhow::Error::new(WorkerFailure::invalid_output()).context("candidate organizer");
        assert_eq!(FailureKind::from_error(&error), FailureKind::InvalidOutput);
        assert_eq!(
            FailureKind::from_message(
                "Waiting for maintenance worker: Infer Runtime maintenance submission timed out: deadline has elapsed"
            ),
            FailureKind::ResponseTimeout
        );
        assert_eq!(
            FailureKind::from_message(
                "decode strict PCP maintenance decision from Infer Runtime: duplicate field `action`"
            ),
            FailureKind::InvalidOutput
        );
        assert_eq!(
            FailureKind::from_message("unrelated storage error"),
            FailureKind::WorkerFailed
        );
    }
}
