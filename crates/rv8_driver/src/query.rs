// CEP:FILE: crates/rv8_driver/src/query.rs
// CEP:WHAT: Stage hashing for the compilation pipeline: records the canonical hash of
//   every IR level per function so determinism is observable, not assumed.
// CEP:WHY: STANDARDS 2.2 requires byte-identical translation; recording per-level hashes
//   turns that requirement into evidence a driver run can print or assert. The full
//   demand-driven memoized query system (mirroring rustc queries) is RV8-134.
// CEP:CLASS: HPC-1
// CEP:STATUS: partial
// CEP:OWNER: axiomzero2
// CEP:TICKET: RV8-134
// CEP:FAILURE: record is total over valid inputs.
// CEP:ASSUMES: canonical_hash is stable for identical IR shapes.
// CEP:COST: O(1) amortized per record.
// CEP:EVIDENCE: unit tests rv8_driver::query::tests::*; pipeline.rs records on every run.

use std::collections::BTreeMap;

/// CEP:WHAT: Per-function record of IR-level canonical hashes.
/// CEP:WHY: One row of evidence per function; printed under --emit run diagnostics and
///   asserted in determinism tests.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: POD.
/// CEP:EVIDENCE: unit test rv8_driver::query::tests::stage_hashes_record_and_compare.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StageHashes {
    /// Function name in declaration order.
    pub function: String,
    /// Hash after flattening (Sea of Nodes).
    pub son: u64,
    /// Hash after middle-end SoN passes.
    pub son_opt: u64,
    /// Hash after linearization (Heterogeneous IR).
    pub het: u64,
    /// Hash after Heterogeneous IR dead-node elimination.
    pub het_opt: u64,
    /// Hash of the final Machine IR before encoding.
    pub mir: u64,
}

/// CEP:WHAT: Ordered collection of stage hashes for one compilation.
/// CEP:WHY: BTreeMap keeps function order deterministic for printing and comparison.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(1) amortized per insert.
/// CEP:EVIDENCE: unit test rv8_driver::query::tests::stage_hashes_record_and_compare.
#[derive(Clone, Debug, Default)]
pub struct StageLog {
    entries: BTreeMap<String, StageHashes>,
}

impl StageLog {
    /// CEP:WHAT: Creates an empty log.
    /// CEP:WHY: Construction boundary.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: this module.
    pub fn new() -> StageLog {
        StageLog { entries: BTreeMap::new() }
    }

    /// CEP:WHAT: Records (or overwrites) the row for one function.
    /// CEP:WHY: The pipeline calls this after each stage completes.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: function names are unique per compilation (frontend enforces).
    /// CEP:COST: O(log n).
    /// CEP:EVIDENCE: unit test rv8_driver::query::tests::stage_hashes_record_and_compare.
    pub fn record(&mut self, row: StageHashes) {
        self.entries.insert(row.function.clone(), row);
    }

    /// CEP:WHAT: Read-only access to all rows in deterministic (name) order.
    /// CEP:WHY: Printing and comparison iterate this vector.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(n) to collect.
    /// CEP:EVIDENCE: unit test rv8_driver::query::tests::stage_hashes_record_and_compare.
    pub fn rows(&self) -> Vec<&StageHashes> {
        self.entries.values().collect()
    }

    /// CEP:WHAT: True when two logs carry identical rows (names and all hashes).
    /// CEP:WHY: The determinism assertion: two pipeline runs must produce equal logs.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(n).
    /// CEP:EVIDENCE: unit test rv8_driver::query::tests::stage_hashes_record_and_compare.
    pub fn equals(&self, other: &StageLog) -> bool {
        self.entries == other.entries
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// CEP:WHAT: Records rows, reads them back in order, compares logs.
    /// CEP:WHY: Core contract of the stage log.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn stage_hashes_record_and_compare() {
        let mut a = StageLog::new();
        a.record(StageHashes {
            function: "main".to_string(),
            son: 1,
            son_opt: 2,
            het: 3,
            het_opt: 4,
            mir: 5,
        });
        a.record(StageHashes {
            function: "add".to_string(),
            son: 6,
            son_opt: 7,
            het: 8,
            het_opt: 9,
            mir: 10,
        });
        let rows = a.rows();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].function, "add", "BTreeMap order is deterministic by name");
        let b = a.clone();
        assert!(a.equals(&b));
        let mut c = StageLog::new();
        c.record(StageHashes {
            function: "add".to_string(),
            son: 6,
            son_opt: 7,
            het: 8,
            het_opt: 9,
            mir: 11,
        });
        assert!(!a.equals(&c), "a differing mir hash must compare unequal");
    }
}
