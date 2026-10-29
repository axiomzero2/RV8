// CEP:FILE: crates/rv8_driver/src/error.rs
// CEP:WHAT: The driver error type: one variant per failed stage plus IO, usage, and
//   execution failures, with stage names preserved for diagnostics.
// CEP:WHY: Fail-closed orchestration: every fallible stage maps into one type so main
//   can render a single, honest message and exit code.
// CEP:CLASS: HPC-1
// CEP:STATUS: complete
// CEP:FAILURE: none
// CEP:ASSUMES: stage messages carry Display strings from the owning crate.
// CEP:COST: allocation on error paths only.
// CEP:EVIDENCE: unit tests rv8_driver::error::tests::*.
use std::fmt;

/// CEP:WHAT: Every way a compilation or run can fail.
/// CEP:WHY: Named failures per STANDARDS 2.3: no panics, every rejection states its cause.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: allocation only on error paths.
/// CEP:EVIDENCE: unit tests rv8_driver::error::tests::display_is_stable.
#[derive(Debug)]
pub enum DriverError {
    /// Malformed CLI invocation.
    Usage(String),
    /// A stage failed; the tag names the pipeline stage, the message the cause.
    Stage { stage: &'static str, message: String },
    /// Filesystem or process failure.
    Io(String),
    /// The linked program ran and returned an exit code outside the expectation.
    ExitMismatch { expected: i32, actual: i32 },
    /// The linked program could not be executed on this platform.
    ExecUnsupported,
}

impl DriverError {
    /// CEP:WHAT: Builds a Usage error from any string-like cause.
    /// CEP:WHY: Keeps parse_args call sites terse.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(message).
    /// CEP:EVIDENCE: used across session.rs.
    pub fn usage(message: impl Into<String>) -> DriverError {
        DriverError::Usage(message.into())
    }
}

impl fmt::Display for DriverError {
    /// CEP:WHAT: Renders the error for stderr.
    /// CEP:WHY: Deterministic, stable diagnostics (38.15).
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(message).
    /// CEP:EVIDENCE: unit test rv8_driver::error::tests::display_is_stable.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DriverError::Usage(m) => write!(f, "usage: {m}"),
            DriverError::Stage { stage, message } => write!(f, "{stage}: {message}"),
            DriverError::Io(m) => write!(f, "io: {m}"),
            DriverError::ExitMismatch { expected, actual } => {
                write!(f, "exit mismatch: expected {expected}, got {actual}")
            }
            DriverError::ExecUnsupported => write!(f, "exec: unsupported platform"),
        }
    }
}

impl std::error::Error for DriverError {}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// CEP:WHAT: Pins the Display rendering of every variant.
    /// CEP:WHY: Diagnostics are part of the contract; changes here are user-visible.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn display_is_stable() {
        assert_eq!(
            DriverError::usage("bad flag").to_string(),
            "usage: bad flag"
        );
        assert_eq!(
            DriverError::Stage { stage: "isel", message: "boom".to_string() }.to_string(),
            "isel: boom"
        );
        assert_eq!(
            DriverError::ExitMismatch { expected: 13, actual: 1 }.to_string(),
            "exit mismatch: expected 13, got 1"
        );
        assert_eq!(DriverError::ExecUnsupported.to_string(), "exec: unsupported platform");
    }
}
