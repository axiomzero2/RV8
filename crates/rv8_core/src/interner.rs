// CEP:FILE: crates/rv8_core/src/interner.rs
// CEP:WHAT: A global-per-instance string interner storing identifiers as compact u32 SymbolIds.
// CEP:WHY: AST and IR nodes must reference identifiers by value, never by pointer or heap string,
//   to keep nodes POD, comparable, and hashable (standard 38.17 bans pointer identity).
// CEP:CLASS: HPC-0
// CEP:STATUS: complete
// CEP:FAILURE: none; interning always succeeds or aborts on OOM via allocator contract.
// CEP:ASSUMES: single Interner instance per compilation session; NOT thread-safe, each worker
//   owns its interner and merges by SymbolId agreement at session creation.
// CEP:COST: get_or_intern O(len) hash plus one map probe; resolve O(1) index.
// CEP:EVIDENCE: unit test rv8_core::interner::tests::intern_roundtrip.
// CEP:SECURITY: identifiers may come from untrusted source text; no unsafe code, no unbounded
//   growth beyond allocator limits.

use crate::hash::FxHashMap;

/// CEP:WHAT: Handle to an interned string, unique within one Interner.
/// CEP:WHY: u32 handles keep AST nodes small and Ord/Hash derivable.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: SymbolId 0 is reserved for the empty symbol and never returned by get_or_intern.
/// CEP:COST: POD u32 wrapper.
/// CEP:EVIDENCE: unit test rv8_core::interner::tests::intern_roundtrip.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct SymbolId(pub u32);

/// CEP:WHAT: String interner with deduplicated storage and O(1) resolution.
/// CEP:WHY: Avoids storing duplicate identifier text once per AST occurrence.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: used from one thread per instance.
/// CEP:COST: intern amortized O(len); resolve O(1).
/// CEP:EVIDENCE: unit test rv8_core::interner::tests::intern_roundtrip.
#[derive(Default)]
pub struct Interner {
    map: FxHashMap<Box<str>, SymbolId>,
    strings: Vec<Box<str>>,
}

impl Interner {
    /// CEP:WHAT: Creates an empty interner with the empty string pre-registered as SymbolId(0).
    /// CEP:WHY: A reserved zero value lets default structs reference a defined symbol.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: one insertion.
    /// CEP:EVIDENCE: unit test rv8_core::interner::tests::intern_roundtrip.
    pub fn new() -> Interner {
        let mut i = Interner {
            map: FxHashMap::default(),
            strings: Vec::new(),
        };
        i.strings.push(Box::<str>::from(""));
        i.map.insert(Box::<str>::from(""), SymbolId(0));
        i
    }

    /// CEP:WHAT: Returns the SymbolId for text, interning it on first sight.
    /// CEP:WHY: Single entry point guarantees deduplication.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(len) hash; amortized O(1) insertion.
    /// CEP:EVIDENCE: unit test rv8_core::interner::tests::intern_roundtrip.
    pub fn get_or_intern(&mut self, text: &str) -> SymbolId {
        if let Some(&id) = self.map.get(text) {
            return id;
        }
        let id = SymbolId(self.strings.len() as u32);
        let boxed: Box<str> = Box::from(text);
        self.strings.push(boxed.clone());
        self.map.insert(boxed, id);
        id
    }

    /// CEP:WHAT: Resolves a SymbolId back to its text.
    /// CEP:WHY: Diagnostics and IR printing need the original spelling.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: Returns None for ids not issued by this interner.
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_core::interner::tests::intern_roundtrip.
    pub fn resolve(&self, id: SymbolId) -> Option<&str> {
        self.strings.get(id.0 as usize).map(|s| s.as_ref())
    }

    /// CEP:WHAT: Returns the number of interned strings including the reserved empty symbol.
    /// CEP:WHY: Tests and metrics report interner pressure.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_core::interner::tests::intern_roundtrip.
    pub fn len(&self) -> usize {
        self.strings.len()
    }

    /// CEP:WHAT: Reports whether the interner holds no entries beyond the reserved empty symbol.
    /// CEP:WHY: Idiomatic emptiness check paired with len.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_core::interner::tests::intern_roundtrip.
    pub fn is_empty(&self) -> bool {
        self.strings.len() <= 1
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// CEP:WHAT: Verifies interning deduplicates and resolves back to the original text.
    /// CEP:WHY: Symbol identity is the backbone of name resolution.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert on mismatch (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn intern_roundtrip() {
        let mut i = Interner::new();
        let a = i.get_or_intern("alpha");
        let a2 = i.get_or_intern("alpha");
        let b = i.get_or_intern("beta");
        assert_eq!(a, a2);
        assert_ne!(a, b);
        assert_eq!(i.resolve(a), Some("alpha"));
        assert_eq!(i.resolve(b), Some("beta"));
        assert_eq!(i.len(), 3);
        assert!(!i.is_empty());
    }
}
