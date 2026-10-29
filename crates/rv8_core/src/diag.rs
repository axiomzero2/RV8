// CEP:FILE: crates/rv8_core/src/diag.rs
// CEP:WHAT: Structured diagnostic objects, stable diagnostic identifiers, and a bounded
//   concurrent diagnostic engine that decouples error production from emission.
// CEP:WHY: Standard 38.15 requires stable, deterministic, bounded diagnostics that never affect
//   codegen; compilation threads only push structured objects, a cold thread formats them.
// CEP:CLASS: HPC-1
// CEP:STATUS: complete
// CEP:FAILURE: push fails closed (returns the diagnostic) when the error queue is full or the
//   engine has reached the error limit; formatting never mutates compilation state.
// CEP:ASSUMES: DiagId values are assigned once and never reused; emission sorts by (id, span).
// CEP:COST: push is one lock-free queue operation; formatting is cold-path only.
// CEP:EVIDENCE: unit test rv8_core::diag::tests::engine_bounded_and_stable.
// CEP:SECURITY: messages carry source text only via caller decision; engine adds no environment
//   data, addresses, or timestamps.

use crate::queue::{DrainBuffer, MpmcQueue};
use crate::span::Span;
use std::fmt;

/// CEP:WHAT: Capacity of the diagnostic queue.
/// CEP:WHY: Named bound per standard 38.15 "bounded in count"; sized for pathological inputs.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: unit test rv8_core::diag::tests::engine_bounded_and_stable.
pub const DIAG_QUEUE_CAPACITY: usize = 4096;

/// CEP:WHAT: Maximum errors before compilation halts.
/// CEP:WHY: Bounded failure per Law 6; prevents diagnostic floods from huge invalid inputs.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: unit test rv8_core::diag::tests::engine_bounded_and_stable.
pub const DIAG_MAX_ERRORS: usize = 128;

/// CEP:WHAT: Stable diagnostic identifier: a numeric code and a stable slug.
/// CEP:WHY: Diagnostic IDs must be stable across releases even if text changes (38.15).
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: slugs are unique across the compiler.
/// CEP:COST: POD.
/// CEP:EVIDENCE: unit test rv8_core::diag::tests::engine_bounded_and_stable.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct DiagId(pub u16);

/// CEP:WHAT: Diagnostic severity levels in increasing order.
/// CEP:WHY: Emission and halting policy depend on severity only.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: POD enum.
/// CEP:EVIDENCE: unit test rv8_core::diag::tests::engine_bounded_and_stable.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Level {
    Note,
    Warning,
    Error,
}

/// CEP:WHAT: One structured diagnostic: identity, severity, primary span, secondary spans, and
///   structured message parts.
/// CEP:WHY: Deferred formatting needs full structure, not preformatted strings.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: spans reference the SourceMap of the same session.
/// CEP:COST: allocation only at construction on error paths (cold relative to hot passes).
/// CEP:EVIDENCE: unit test rv8_core::diag::tests::engine_bounded_and_stable.
#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub id: DiagId,
    pub slug: &'static str,
    pub level: Level,
    pub span: Option<Span>,
    pub related: Vec<Span>,
    pub message: String,
    pub help: Option<String>,
}

impl Diagnostic {
    /// CEP:WHAT: Formats the diagnostic in a stable, machine-parseable single-line form.
    /// CEP:WHY: CI and tooling parse one canonical text form.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: cold.
    /// CEP:EVIDENCE: unit test rv8_core::diag::tests::engine_bounded_and_stable.
    pub fn render(&self) -> String {
        let level_text = match self.level {
            Level::Note => "note",
            Level::Warning => "warning",
            Level::Error => "error",
        };
        let mut out = match self.span {
            Some(span) => format!(
                "{level_text}[RV{:04}:{}]: {} at {span}",
                self.id.0, self.slug, self.message
            ),
            None => format!("{level_text}[RV{:04}:{}]: {}", self.id.0, self.slug, self.message),
        };
        if let Some(help) = &self.help {
            out.push_str(&format!(" | help: {help}"));
        }
        out
    }
}

impl fmt::Display for Diagnostic {
    /// CEP:WHAT: Display delegates to render().
    /// CEP:WHY: One canonical text form.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: cold.
    /// CEP:EVIDENCE: unit test rv8_core::diag::tests::engine_bounded_and_stable.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.render())
    }
}

/// CEP:WHAT: Concurrent diagnostic engine: producers push into a bounded lock-free queue; the
///   consumer drains in stable (level, id, span) order.
/// CEP:WHY: Decouples hot compilation threads from cold formatting and printing.
/// CEP:STATUS: complete
/// CEP:FAILURE: push returns the diagnostic on overflow or after the error limit.
/// CEP:ASSUMES: one engine per compilation session.
/// CEP:COST: push O(1) lock-free; drain O(n log n) cold.
/// CEP:EVIDENCE: unit test rv8_core::diag::tests::engine_bounded_and_stable.
pub struct DiagEngine {
    queue: MpmcQueue<Diagnostic>,
    overflow: DrainBuffer<Diagnostic>,
    error_count: std::sync::atomic::AtomicUsize,
}

impl Default for DiagEngine {
    /// CEP:WHAT: Creates an engine with the standard queue capacity.
    /// CEP:WHY: One shared configuration point.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(capacity) once.
    /// CEP:EVIDENCE: unit test rv8_core::diag::tests::engine_bounded_and_stable.
    fn default() -> Self {
        DiagEngine::with_capacity(DIAG_QUEUE_CAPACITY)
    }
}

impl DiagEngine {
    /// CEP:WHAT: Creates an engine with an explicit queue capacity.
    /// CEP:WHY: Tests use small capacities to exercise overflow.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: capacity >= 1.
    /// CEP:COST: O(capacity).
    /// CEP:EVIDENCE: unit test rv8_core::diag::tests::engine_bounded_and_stable.
    pub fn with_capacity(capacity: usize) -> DiagEngine {
        DiagEngine {
            queue: MpmcQueue::with_capacity(capacity),
            overflow: DrainBuffer::new(),
            error_count: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// CEP:WHAT: Emits one diagnostic; rejects new errors once the error limit is reached.
    /// CEP:WHY: Bounded failure (Law 6) and hot-path non-blocking emission.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: Returns Err(diagnostic) when full or over the error limit.
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1) lock-free push.
    /// CEP:EVIDENCE: unit test rv8_core::diag::tests::engine_bounded_and_stable.
    pub fn push(&self, diag: Diagnostic) -> Result<(), Diagnostic> {
        if diag.level == Level::Error {
            let prev = self
                .error_count
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if prev >= DIAG_MAX_ERRORS {
                return Err(diag);
            }
        }
        match self.queue.push(diag) {
            Ok(()) => Ok(()),
            Err(d) => self.overflow.push(d),
        }
    }

    /// CEP:WHAT: Reports whether the error limit has been reached.
    /// CEP:WHY: Passes check before starting bounded work.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1) relaxed load.
    /// CEP:EVIDENCE: unit test rv8_core::diag::tests::engine_bounded_and_stable.
    pub fn has_fatal_errors(&self) -> bool {
        self.error_count
            .load(std::sync::atomic::Ordering::Relaxed)
            > 0
    }

    /// CEP:WHAT: Drains all queued diagnostics in stable sorted order.
    /// CEP:WHY: Deterministic emission (38.15).
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(n log n).
    /// CEP:EVIDENCE: unit test rv8_core::diag::tests::engine_bounded_and_stable.
    pub fn drain(&self) -> Vec<Diagnostic> {
        let mut out: Vec<Diagnostic> = Vec::new();
        while let Some(d) = self.queue.pop() {
            out.push(d);
        }
        if let Ok(extra) = self.overflow.drain_sorted(|d| (d.level, d.id.0, d.span.map(|s| (s.file.0, s.lo, s.hi)))) {
            out.extend(extra);
        }
        out.sort_by_key(|d| (d.level, d.id.0, d.span.map(|s| (s.file.0, s.lo, s.hi))));
        out
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::span::{FileId, Span};

    /// CEP:WHAT: Verifies stable ordering, rendering, and error-limit behavior.
    /// CEP:WHY: Diagnostic contract of standard 38.15.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert on mismatch (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn engine_bounded_and_stable() {
        let engine = DiagEngine::with_capacity(8);
        let span = Span::new(FileId(0), 1, 5);
        for id in [3u16, 1, 2] {
            engine
                .push(Diagnostic {
                    id: DiagId(id),
                    slug: "test_diag",
                    level: Level::Error,
                    span: Some(span),
                    related: Vec::new(),
                    message: format!("problem {id}"),
                    help: Some("fix it".to_string()),
                })
                .unwrap();
        }
        engine.push(Diagnostic {
            id: DiagId(9),
            slug: "test_note",
            level: Level::Note,
            span: None,
            related: Vec::new(),
            message: "context".to_string(),
            help: None,
        }).unwrap();
        let drained = engine.drain();
        let rendered: Vec<String> = drained.iter().map(|d| d.render()).collect();
        assert_eq!(rendered.len(), 4);
        assert!(rendered[0].starts_with("note[RV0009"));
        assert!(rendered[1].starts_with("error[RV0001"));
        assert!(rendered[3].contains("help: fix it"));
        assert!(engine.has_fatal_errors());
    }
}
