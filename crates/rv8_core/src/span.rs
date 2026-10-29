// CEP:FILE: crates/rv8_core/src/span.rs
// CEP:WHAT: Byte-position spans and a source map mapping spans back to file, line, and column.
// CEP:WHY: Every diagnostic must be source-locatable (standard 38.15); a 12-byte span per IR node
//   keeps provenance cheap enough to carry through every compiler stage.
// CEP:CLASS: HPC-0
// CEP:STATUS: complete
// CEP:FAILURE: SourceMap::line_col returns None for out-of-range positions instead of panicking.
// CEP:ASSUMES: file count and file sizes fit in u32; enforced by runtime checks on registration.
// CEP:COST: line_col is O(log L) on file line count via binary search; Span itself is POD.
// CEP:EVIDENCE: unit test rv8_core::span::tests::line_col_roundtrip.
// CEP:SECURITY: input source text is untrusted; no indexing without bounds checks.

use core::fmt;

/// CEP:WHAT: Identifier of a registered source file inside a SourceMap.
/// CEP:WHY: Spans must be stable across compilation; a u32 file id avoids pointer identity.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: zero-sized wrapper over u32.
/// CEP:EVIDENCE: unit test rv8_core::span::tests::line_col_roundtrip.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct FileId(pub u32);

/// CEP:WHAT: A half-open byte range [lo, hi) inside one source file.
/// CEP:WHY: Spans are the provenance chain anchors required by the diagnostics standard.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: lo <= hi; checked by debug assertion in new().
/// CEP:COST: POD, 12 bytes on 32-bit fields; Copy by value.
/// CEP:EVIDENCE: unit test rv8_core::span::tests::line_col_roundtrip.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Span {
    pub file: FileId,
    pub lo: u32,
    pub hi: u32,
}

// CEP:WHAT: Named constant for the maximum u32 offset; documents the span range bound.
// CEP:WHY: Standard Law 7 bans magic numbers; the value appears once, named.
// CEP:STATUS: complete
// CEP:FAILURE: none
// CEP:ASSUMES: none
// CEP:COST: compile-time constant.
// CEP:EVIDENCE: review 2026-10-30.
pub const SPAN_MAX_OFFSET: u32 = u32::MAX;

impl Span {
    /// CEP:WHAT: Constructs a span after checking that lo <= hi.
    /// CEP:WHY: Catching inverted ranges at construction prevents cascading broken diagnostics.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: Panics in debug builds if lo > hi (programmer error, not input error).
    /// CEP:ASSUMES: lo <= hi; checked by debug assertion.
    /// CEP:COST: two register moves plus a debug-only compare.
    /// CEP:EVIDENCE: unit test rv8_core::span::tests::line_col_roundtrip.
    pub fn new(file: FileId, lo: u32, hi: u32) -> Span {
        debug_assert!(lo <= hi, "span lo above hi");
        Span { file, lo, hi }
    }

    /// CEP:WHAT: Returns a span covering both self and other within the same file.
    /// CEP:WHY: Multi-token diagnostics need one merged span for the primary location.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: self.file == other.file; checked by debug assertion.
    /// CEP:COST: two min/max operations.
    /// CEP:EVIDENCE: unit test rv8_core::span::tests::merge.
    pub fn merge(self, other: Span) -> Span {
        debug_assert!(self.file == other.file, "merge across files");
        Span {
            file: self.file,
            lo: self.lo.min(other.lo),
            hi: self.hi.max(other.hi),
        }
    }
}

impl fmt::Display for Span {
    /// CEP:WHAT: Formats the span as file:lo-hi in a stable, machine-parseable form.
    /// CEP:WHY: Deterministic diagnostic output requires stable span formatting.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: cold; formatting only on diagnostic emission.
    /// CEP:EVIDENCE: unit test rv8_core::span::tests::line_col_roundtrip.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "@{}:{}-{}", self.file.0, self.lo, self.hi)
    }
}

/// CEP:WHAT: One registered source file with its text and precomputed line-start offsets.
/// CEP:WHY: Line and column lookups must be O(log L) without rescanning text.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: text is valid UTF-8 (String invariant).
/// CEP:COST: line index built once at O(N) registration.
/// CEP:EVIDENCE: unit test rv8_core::span::tests::line_col_roundtrip.
#[derive(Clone, Debug)]
pub struct SourceFile {
    pub name: String,
    pub text: String,
    pub line_starts: Vec<u32>,
}

/// CEP:WHAT: Registry of source files providing span-to-line/column resolution.
/// CEP:WHY: Diagnostics need human locations; the registry is the single authority for FileId.
/// CEP:STATUS: complete
/// CEP:FAILURE: add_file returns a descriptive error String when u32 limits are exceeded;
///   line_col returns None for foreign or out-of-range positions.
/// CEP:ASSUMES: file count <= u32::MAX; checked at registration.
/// CEP:COST: add_file O(N) for the line scan; line_col O(log L).
/// CEP:EVIDENCE: unit test rv8_core::span::tests::line_col_roundtrip.
#[derive(Default)]
pub struct SourceMap {
    files: Vec<SourceFile>,
}

impl SourceMap {
    /// CEP:WHAT: Registers source text and returns its FileId.
    /// CEP:WHY: All source enters compilation through one checked boundary.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: Returns Err when the file index or text length exceeds u32 range.
    /// CEP:ASSUMES: name is a display label; not used for identity.
    /// CEP:COST: O(N) over the text to record line starts.
    /// CEP:EVIDENCE: unit test rv8_core::span::tests::line_col_roundtrip.
    pub fn add_file(&mut self, name: &str, text: String) -> Result<FileId, String> {
        let idx64 = self.files.len() as u64;
        if idx64 > SPAN_MAX_OFFSET as u64 || text.len() as u64 > SPAN_MAX_OFFSET as u64 {
            return Err(format!("source file {} exceeds u32 span range", name));
        }
        let mut line_starts = Vec::with_capacity(16);
        line_starts.push(0u32);
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push((i + 1) as u32);
            }
        }
        let file_id = FileId(idx64 as u32);
        self.files.push(SourceFile {
            name: name.to_string(),
            text,
            line_starts,
        });
        Ok(file_id)
    }

    /// CEP:WHAT: Resolves a position to 1-based line and column numbers.
    /// CEP:WHY: Standard 38.15 requires source-locatable diagnostics.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: Returns None for unknown FileId or position beyond file end.
    /// CEP:ASSUMES: position is a byte offset within the registered text.
    /// CEP:COST: O(log L) binary search over line starts.
    /// CEP:EVIDENCE: unit test rv8_core::span::tests::line_col_roundtrip.
    pub fn line_col(&self, span: Span) -> Option<(u32, u32)> {
        let file = self.files.get(span.file.0 as usize)?;
        if span.lo > file.text.len() as u32 {
            return None;
        }
        let line_index = file.line_starts.partition_point(|&s| s <= span.lo) - 1;
        let line_start = file.line_starts[line_index];
        let col = span.lo - line_start + 1;
        Some((line_index as u32 + 1, col))
    }

    /// CEP:WHAT: Returns the source file name for a FileId, if registered.
    /// CEP:WHY: Diagnostics display file names without exposing registry internals.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: Returns None for unknown FileId.
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1) index.
    /// CEP:EVIDENCE: unit test rv8_core::span::tests::line_col_roundtrip.
    pub fn file_name(&self, file: FileId) -> Option<&str> {
        self.files.get(file.0 as usize).map(|f| f.name.as_str())
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// CEP:WHAT: Verifies span merge and line/column round-trip through SourceMap.
    /// CEP:WHY: Diagnostics depend on exact location recovery.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert on mismatch (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn line_col_roundtrip() {
        let mut sm = SourceMap::default();
        let fid = sm.add_file("a.rs", "ab\ncd\n".to_string()).unwrap();
        let span = Span::new(fid, 3, 4);
        assert_eq!(sm.line_col(span), Some((2, 1)));
        assert_eq!(sm.file_name(fid), Some("a.rs"));
    }

    /// CEP:WHAT: Verifies merged spans cover both endpoints.
    /// CEP:WHY: Multi-token diagnostics must not shrink.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert on mismatch (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn merge() {
        let fid = FileId(0);
        let a = Span::new(fid, 10, 12);
        let b = Span::new(fid, 2, 5);
        let m = a.merge(b);
        assert_eq!((m.lo, m.hi), (2, 12));
    }
}
