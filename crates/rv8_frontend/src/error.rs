// CEP:FILE: crates/rv8_frontend/src/error.rs
// CEP:WHAT: The FrontendError contract: one fail-closed error type covering lexing, parsing,
//   name resolution, type inference, lowering, and verification failures, with a source Span
//   on every variant so every diagnostic is locatable (standard 38.15).
// CEP:WHY: Library code must never panic on untrusted input (38.6); the pipeline reports the
//   first rejection as a typed, spanned value instead.
// CEP:CLASS: HPC-0
// CEP:STATUS: complete
// CEP:FAILURE: none; this type IS the failure contract of the frontend.
// CEP:ASSUMES: spans reference the single source file registered for one compile_source call.
// CEP:COST: allocation only on error paths (cold); construction is O(1) plus message text.
// CEP:EVIDENCE: unit tests rv8_frontend::lib::tests::compile_source_rejects_*.

use rv8_core::span::Span;
use rv8_ir::son::IrType;
use std::fmt;

/// CEP:WHAT: Classification of lexical failures.
/// CEP:WHY: One FrontendError::Lex variant carries a structured kind instead of free text so
///   callers can match rejection classes exactly (one negative test per class).
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: POD.
/// CEP:EVIDENCE: unit tests rv8_frontend::lexer::tests::*.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LexErrorKind {
    UnexpectedChar,
    UnterminatedBlockComment,
    UnclosedDelimiter { open: char },
    MismatchedDelimiter { open: char, found: char },
    StrayClose { found: char },
    IntLiteralOverflow,
    LiteralSuffixUnsupported,
}

/// CEP:WHAT: Classification of syntactic failures.
/// CEP:WHY: Structured parse errors keep tests precise and messages uniform.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: POD.
/// CEP:EVIDENCE: unit tests rv8_frontend::parser::tests::*.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseErrorKind {
    UnexpectedToken { expected: &'static str },
    UnexpectedEof { expected: &'static str },
}

/// CEP:WHAT: The single error type of the frontend pipeline; every variant carries a Span.
/// CEP:WHY: Standard 38.6 and 38.15 require fail-closed, source-locatable rejection of every
///   malformed input class with no panics reachable from library code.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; this type enumerates the failures.
/// CEP:ASSUMES: none
/// CEP:COST: allocation only on the cold error path.
/// CEP:EVIDENCE: unit tests rv8_frontend::lib::tests::compile_source_rejects_*.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FrontendError {
    Lex { kind: LexErrorKind, span: Span },
    Parse { kind: ParseErrorKind, span: Span },
    NestingTooDeep { span: Span },
    SourceTooLarge { len: usize, span: Span },
    AstTooLarge { span: Span },
    DuplicateFunction { name: String, span: Span },
    UnknownFunction { name: String, span: Span },
    ArityMismatch { name: String, expected: usize, found: usize, span: Span },
    UnknownVariable { name: String, span: Span },
    AssignToImmutable { name: String, span: Span },
    TypeMismatch { context: &'static str, expected: IrType, found: IrType, span: Span },
    OperandTypeInvalid { requirement: &'static str, found: IrType, span: Span },
    UnresolvedType { span: Span },
    BreakOutsideLoop { span: Span },
    ContinueOutsideLoop { span: Span },
    Unsupported { feature: &'static str, span: Span },
    VerificationFailed { detail: String, span: Span },
    Internal { context: &'static str, span: Span },
}

impl fmt::Display for FrontendError {
    /// CEP:WHAT: Renders the error in one stable, human-readable line.
    /// CEP:WHY: Deterministic diagnostics (38.15) need one canonical text form.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: cold-path formatting only.
    /// CEP:EVIDENCE: unit test rv8_frontend::lib::tests::compile_source_is_deterministic.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FrontendError::Lex { kind, span } => match kind {
                LexErrorKind::UnexpectedChar => write!(f, "unexpected character {span}"),
                LexErrorKind::UnterminatedBlockComment => {
                    write!(f, "block comment reaches end of file {span}")
                }
                LexErrorKind::UnclosedDelimiter { open } => {
                    write!(f, "delimiter `{open}` is never closed {span}")
                }
                LexErrorKind::MismatchedDelimiter { open, found } => {
                    write!(f, "delimiter `{found}` closes `{open}` {span}")
                }
                LexErrorKind::StrayClose { found } => {
                    write!(f, "closing delimiter `{found}` with no open group {span}")
                }
                LexErrorKind::IntLiteralOverflow => {
                    write!(f, "integer literal does not fit in i64 {span}")
                }
                LexErrorKind::LiteralSuffixUnsupported => {
                    write!(f, "integer literal suffixes are not supported {span}")
                }
            },
            FrontendError::Parse { kind, span } => match kind {
                ParseErrorKind::UnexpectedToken { expected } => {
                    write!(f, "expected {expected} {span}")
                }
                ParseErrorKind::UnexpectedEof { expected } => {
                    write!(f, "expected {expected} but reached end of file {span}")
                }
            },
            FrontendError::NestingTooDeep { span } => {
                write!(f, "nesting exceeds the parse depth budget {span}")
            }
            FrontendError::SourceTooLarge { len, span } => {
                write!(f, "source of {len} bytes exceeds the span range {span}")
            }
            FrontendError::AstTooLarge { span } => {
                write!(f, "program exceeds a frontend size budget {span}")
            }
            FrontendError::DuplicateFunction { name, span } => {
                write!(f, "duplicate function `{name}` {span}")
            }
            FrontendError::UnknownFunction { name, span } => {
                write!(f, "call to unknown function `{name}` {span}")
            }
            FrontendError::ArityMismatch { name, expected, found, span } => {
                write!(f, "function `{name}` expects {expected} arguments, found {found} {span}")
            }
            FrontendError::UnknownVariable { name, span } => {
                write!(f, "unknown variable `{name}` {span}")
            }
            FrontendError::AssignToImmutable { name, span } => {
                write!(f, "cannot assign to immutable variable `{name}` {span}")
            }
            FrontendError::TypeMismatch { context, expected, found, span } => {
                write!(f, "type mismatch in {context}: expected {expected:?}, found {found:?} {span}")
            }
            FrontendError::OperandTypeInvalid { requirement, found, span } => {
                write!(f, "operand must be {requirement}, found {found:?} {span}")
            }
            FrontendError::UnresolvedType { span } => {
                write!(f, "type of expression cannot be determined {span}")
            }
            FrontendError::BreakOutsideLoop { span } => write!(f, "`break` outside of loop {span}"),
            FrontendError::ContinueOutsideLoop { span } => {
                write!(f, "`continue` outside of loop {span}")
            }
            FrontendError::Unsupported { feature, span } => {
                write!(f, "unsupported construct: {feature} {span}")
            }
            FrontendError::VerificationFailed { detail, span } => {
                write!(f, "HIR verification failed: {detail} {span}")
            }
            FrontendError::Internal { context, span } => {
                write!(f, "internal invariant violation ({context}) {span}")
            }
        }
    }
}

impl std::error::Error for FrontendError {}
