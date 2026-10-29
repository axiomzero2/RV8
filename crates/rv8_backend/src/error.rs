// CEP:FILE: crates/rv8_backend/src/error.rs
// CEP:WHAT: The single error type for every fallible backend operation: instruction
//   selection, MCF register allocation, spill rewriting, scheduling, and x86-64 encoding.
// CEP:WHY: Standard 38.6 forbids panics in library code; one enum keeps `Result` uniform
//   across the pipeline and lets rv8_driver surface backend failures without unwraps.
// CEP:CLASS: HPC-0
// CEP:STATUS: complete
// CEP:FAILURE: none; the type itself is infallible, variants name the failing stage.
// CEP:ASSUMES: detail strings are diagnostic only and must not be parsed by callers;
//   determinism guarantees the same failure on the same input, but text may evolve.
// CEP:COST: allocation only on the error path (String fields).
// CEP:EVIDENCE: unit tests across rv8_backend exercise every non-Internal variant; the
//   Internal variant is unreachable by construction and covered by contract comments.

use std::fmt;

/// CEP:WHAT: Backend pipeline error: one variant per failure class, never a panic.
/// CEP:WHY: Fail-closed contracts (38.6); the pipeline in lib.rs threads this type through
///   select -> allocate -> rewrite -> schedule -> verify -> emit_function.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: LIR handed to the backend satisfies rv8_ir::lir invariants 4.3.1-4.3.4;
///   violations surface as `IllegalLir` rather than panics.
/// CEP:COST: allocation only on error paths.
/// CEP:EVIDENCE: negative tests `encode_rejects_invalid_scale`,
///   `encode_rejects_non_x86_isa`, `isel_rejects_use_before_def`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BackendError {
    /// CEP:WHAT: A memory operand violates the x86-64 addressing contract (scale not in
    /// 1/2/4/8, rsp used as index, displacement out of i32).
    /// CEP:WHY: Mirrors machir invariant 5.3.4 as defense in depth at encoding time.
    /// CEP:STATUS: complete
    InvalidAddressing { detail: String },
    /// CEP:WHAT: The MIR targets an ISA this encoder does not implement (x86-64 only).
    /// CEP:WHY: The 5.3.1 gate must also be enforced by the consumer of verified MIR.
    /// CEP:STATUS: complete
    UnsupportedIsa { isa: &'static str },
    /// CEP:WHAT: An instruction's operand count or operand kinds do not fit its opcode.
    /// CEP:WHY: The encoder is total over well-shaped MIR; bad shapes are rejected, never
    ///   guessed at.
    /// CEP:STATUS: complete
    IllegalOperandShape { opcode: &'static str, detail: String },
    /// CEP:WHAT: A LIR construct has no target pattern (excess stack parameters, floating
    ///   point parameters, vector register class).
    /// CEP:WHY: Honest capability reporting instead of silent mistranslation.
    /// CEP:STATUS: complete
    UnsupportedPattern { detail: String },
    /// CEP:WHAT: The LIR violates a level-3 invariant (use before def in a block,
    ///   missing terminator, multiple definition of one value).
    /// CEP:WHY: The backend refuses corrupt input instead of miscompiling it.
    /// CEP:STATUS: complete
    IllegalLir { detail: String },
    /// CEP:WHAT: A named compile budget was exceeded (value count, network size,
    ///   iteration cap).
    /// CEP:WHY: Standard 38.12: limits are named constants and enforced, not implicit.
    /// CEP:STATUS: complete
    BudgetExceeded { budget: &'static str },
    /// CEP:WHAT: An invariant the algorithms guarantee by construction did not hold.
    /// CEP:WHY: Distinguishes "user program problem" from "compiler bug"; an Internal is
    ///   always a bug report, never a user-facing failure.
    /// CEP:STATUS: complete
    Internal { detail: String },
    /// CEP:WHAT: Machine-IR verification failed after a backend transformation.
    /// CEP:WHY: Standard 38.18: verify after every pass; failures propagate, not panic.
    /// CEP:STATUS: complete
    Verification { detail: String },
}

impl fmt::Display for BackendError {
    /// CEP:WHAT: Renders the error with the failing stage and diagnostic detail.
    /// CEP:WHY: Printable diagnostics per the documentation standard.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(detail).
    /// CEP:EVIDENCE: exercised by every negative test's `.unwrap_err()` message checks.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BackendError::InvalidAddressing { detail } => write!(f, "invalid addressing mode: {detail}"),
            BackendError::UnsupportedIsa { isa } => write!(f, "unsupported ISA for x86-64 encoder: {isa}"),
            BackendError::IllegalOperandShape { opcode, detail } => {
                write!(f, "illegal operand shape for {opcode}: {detail}")
            }
            BackendError::UnsupportedPattern { detail } => write!(f, "unsupported pattern: {detail}"),
            BackendError::IllegalLir { detail } => write!(f, "illegal LIR input: {detail}"),
            BackendError::BudgetExceeded { budget } => write!(f, "compile budget exceeded: {budget}"),
            BackendError::Internal { detail } => write!(f, "internal compiler error: {detail}"),
            BackendError::Verification { detail } => write!(f, "machine IR verification failed: {detail}"),
        }
    }
}

impl std::error::Error for BackendError {
    // Marker-trait impl: Display carries all diagnostic content by design.
}
