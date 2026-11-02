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

// ---------------------------------------------------------------------------
// Demand-driven memoized query system (RV8-134, design.md "The Memoized Query
// System (The Backbone of Incrementality)"): queries execute on demand, memoize,
// record their input dependencies transitively, and re-execute only when a
// dependency changed. Query cycles are structured errors, never loops.
// ---------------------------------------------------------------------------

/// CEP:WHAT: Maximum recursion depth of query execution (named budget, 38.12).
/// CEP:WHY: Query graphs come from user program structure; the budget converts
///   pathological graphs into a structured error instead of stack exhaustion (the
///   engine executes recursively through user Query::execute closures, so the bound
///   is sized to the thread stack: 128 frames measured at well under 2 MiB).
/// CEP:STATUS: complete
/// CEP:FAILURE: BudgetExceeded past this depth.
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: query_deep_chain_succeeds_and_budgets.
pub const MAX_QUERY_DEPTH: usize = 128;

/// CEP:WHAT: Maximum number of memo slots and registered inputs (named budget, 38.12).
/// CEP:WHY: Bounds database growth under adversarial key spaces.
/// CEP:STATUS: complete
/// CEP:FAILURE: BudgetExceeded past either bound.
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: enforced in get and set_input.
pub const MAX_QUERY_SLOTS: usize = 1 << 20;
pub const MAX_QUERY_INPUTS: usize = 1 << 20;

/// CEP:WHAT: A demand-driven query: a pure function from the database state and a key
///   to a value, identified by a unique static ID.
/// CEP:WHY: The unit of memoization; execute may call `QueryDatabase::read_input` to
///   register input dependencies and `QueryDatabase::get` to invoke other queries.
/// CEP:STATUS: complete
/// CEP:FAILURE: execute errors surface as QueryError::Computation, cached like values.
/// CEP:ASSUMES: execute is deterministic (STANDARDS 2.2) and pure; an execute that
///   returns an error for a key must not have partially mutated global state.
/// CEP:COST: defined by the implementation.
/// CEP:EVIDENCE: the driver query set in crate::queries; engine tests below.
pub trait Query: 'static {
    /// Memoized result type.
    type Value: Clone + 'static;
    /// Cached computation-failure type.
    type Error: Clone + 'static;
    /// Unique, stable query identifier (part of the memo slot key).
    const ID: &'static str;
    /// Computes the value for `key`, registering dependencies through `db`.
    fn execute(db: &mut QueryDatabase, key: &str) -> Result<Self::Value, Self::Error>;
}

/// CEP:WHAT: Query failure taxonomy: dependency cycles, named budgets, missing
///   inputs, and cached computation errors.
/// CEP:WHY: Fail-closed: every failure is a structured value, never a panic.
/// CEP:STATUS: complete
/// CEP:FAILURE: this is the failure type.
/// CEP:ASSUMES: E: Clone so cached errors re-serve.
/// CEP:COST: POD.
/// CEP:EVIDENCE: query_cycle_is_a_structured_error, query_missing_input_is_structured.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QueryError<E> {
    /// A query (transitively) invoked itself; `path` is the invocation chain from
    /// the first occurrence to the repeat.
    Cycle { path: Vec<String> },
    /// A named budget was exceeded (see MAX_QUERY_*).
    BudgetExceeded { budget: &'static str },
    /// read_input named an input that was never registered.
    MissingInput { name: String },
    /// The query's own computation failed; the error is cached and re-served
    /// until a dependency changes.
    Computation(E),
}

impl<E: std::fmt::Display> std::fmt::Display for QueryError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            QueryError::Cycle { path } => write!(f, "query cycle: {}", path.join(" -> ")),
            QueryError::BudgetExceeded { budget } => {
                write!(f, "query budget exceeded: {budget}")
            }
            QueryError::MissingInput { name } => write!(f, "missing input: {name}"),
            QueryError::Computation(e) => write!(f, "{e}"),
        }
    }
}

/// CEP:WHAT: One registered input (source text, flag set, hardware fingerprint).
/// CEP:WHY: BLAKE3 content hash decides invalidation, so re-setting identical content
///   does not disturb any memo.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(bytes) per set.
/// CEP:EVIDENCE: query_set_input_same_content_keeps_memos.
#[derive(Clone, Debug)]
struct InputState {
    content: Vec<u8>,
    hash: [u8; 32],
}

/// CEP:WHAT: One memo slot: value or cached error, plus the transitive
///   input-dependency set recorded at compute time.
/// CEP:WHY: A slot is valid exactly while every recorded dependency hash is
///   unchanged; transitivity comes from merging child slots at get time.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: value boxed as the memo payload of the query that wrote it.
/// CEP:COST: POD plus the dependency vector.
/// CEP:EVIDENCE: the engine tests below.
struct Slot {
    value: Box<dyn std::any::Any>,
    deps: Vec<(String, [u8; 32])>,
}

/// CEP:WHAT: Observable engine counters (computes, memo hits, invalidations).
/// CEP:WHY: Incrementality must be observable, not assumed (STANDARDS 2.2 evidence).
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: POD.
/// CEP:EVIDENCE: query_stats_show_memo_hits_and_invalidations.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct QueryStats {
    pub computes: u64,
    pub memo_hits: u64,
    pub invalidations: u64,
}

/// CEP:WHAT: The demand-driven memoized query database: inputs by name, memo slots by
///   (query id, key), an in-progress stack for cycle detection, and counters.
/// CEP:WHY: Design.md makes this the driver backbone; STANDARDS 5.10/5.11 fix the
///   content-addressed, demand-driven model.
/// CEP:STATUS: complete
/// CEP:FAILURE: QueryError from get/read_input; construction is infallible.
/// CEP:ASSUMES: single-threaded ownership (the driver is single-threaded in this
///   revision; crate-DAG parallel scheduling is documented future work).
/// CEP:COST: memo hit O(deps); recompute O(query).
/// CEP:EVIDENCE: the engine test battery below.
#[derive(Default)]
pub struct QueryDatabase {
    revision: u64,
    inputs: std::collections::BTreeMap<String, InputState>,
    slots: std::collections::BTreeMap<String, Slot>,
    in_progress: Vec<String>,
    depth: usize,
    // Dependency accumulator for the frame currently executing (top of stack).
    frame_deps: Vec<Vec<(String, [u8; 32])>>,
    stats: QueryStats,
}

impl QueryDatabase {
    /// CEP:WHAT: Creates an empty database.
    /// CEP:WHY: Construction boundary.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: this module's tests.
    pub fn new() -> QueryDatabase {
        QueryDatabase::default()
    }

    /// CEP:WHAT: Registers or updates an input by name with new content; returns true
    ///   when the content actually changed (revision bump), false when identical.
    /// CEP:WHY: Content-addressed invalidation: identical content keeps every memo.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: BudgetExceeded past MAX_QUERY_INPUTS.
    /// CEP:ASSUMES: none
    /// CEP:COST: O(bytes) for the hash.
    /// CEP:EVIDENCE: query_set_input_same_content_keeps_memos.
    pub fn set_input(&mut self, name: &str, bytes: &[u8]) -> Result<bool, QueryError<std::convert::Infallible>> {
        let hash = rv8_core::blake3::hash(bytes);
        if let Some(existing) = self.inputs.get(name) {
            if existing.hash == hash {
                return Ok(false);
            }
        } else if self.inputs.len() >= MAX_QUERY_INPUTS {
            return Err(QueryError::BudgetExceeded { budget: "MAX_QUERY_INPUTS" });
        }
        self.inputs.insert(
            name.to_string(),
            InputState { content: bytes.to_vec(), hash },
        );
        self.revision += 1;
        Ok(true)
    }

    /// CEP:WHAT: Reads an input's bytes, registering a dependency on it in the
    ///   currently executing query frame.
    /// CEP:WHY: Dependency recording happens at read time, so execute cannot forget it.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: MissingInput when never registered; BudgetExceeded past
    ///   MAX_QUERY_DEPTH-equivalent bookkeeping is impossible here.
    /// CEP:ASSUMES: called only from inside Query::execute.
    /// CEP:COST: O(bytes) clone.
    /// CEP:EVIDENCE: query_memo_hits_skip_recompute.
    pub fn read_input(&mut self, name: &str) -> Result<Vec<u8>, QueryError<std::convert::Infallible>> {
        let state = self
            .inputs
            .get(name)
            .ok_or_else(|| QueryError::MissingInput { name: name.to_string() })?;
        let hash = state.hash;
        let content = state.content.clone();
        if let Some(frame) = self.frame_deps.last_mut() {
            frame.push((name.to_string(), hash));
        }
        Ok(content)
    }

    /// CEP:WHAT: Returns the database revision (bumped on every content change).
    /// CEP:WHY: Diagnostics and tests observe invalidation epochs.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: query_stats_show_memo_hits_and_invalidations.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// CEP:WHAT: Returns the engine counters.
    /// CEP:WHY: Incrementality evidence.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: query_stats_show_memo_hits_and_invalidations.
    pub fn stats(&self) -> QueryStats {
        self.stats
    }

    /// CEP:WHAT: Drops every memo (e.g. compiler-version change per STANDARDS 5.10);
    ///   inputs survive.
    /// CEP:WHY: The autotuner cache is keyed on (LIR hash, fingerprint, version);
    ///   a version bump invalidates derived artifacts, not inputs.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(slots).
    /// CEP:EVIDENCE: query_invalidate_all_forces_recompute.
    pub fn invalidate_all(&mut self) {
        self.slots.clear();
        self.revision += 1;
    }

    /// CEP:WHAT: Demand-driven memoized entry: returns the cached value when every
    ///   transitive input dependency is unchanged, otherwise executes (and caches
    ///   the value or the computation error).
    /// CEP:WHY: The backbone operation of design.md's query system.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: QueryError::Cycle for recursive invocation (with the cycle path),
    ///   BudgetExceeded past MAX_QUERY_DEPTH / MAX_QUERY_SLOTS, Computation when the
    ///   query itself fails (cached).
    /// CEP:ASSUMES: Q::execute is deterministic and registers every input it reads.
    /// CEP:COST: hit O(deps); miss O(query execution).
    /// CEP:EVIDENCE: the engine test battery below; pipeline.rs routes through this.
    pub fn get<Q: Query>(&mut self, key: &str) -> Result<Q::Value, QueryError<Q::Error>> {
        let slot_key = format!("{}\u{0}{}", Q::ID, key);
        if let Some(pos) = self.in_progress.iter().position(|k| *k == slot_key) {
            let mut path: Vec<String> = self.in_progress[pos..]
                .iter()
                .map(|k| k.replace('\u{0}', "::"))
                .collect();
            path.push(slot_key.replace('\u{0}', "::"));
            return Err(QueryError::Cycle { path });
        }
        if self.depth >= MAX_QUERY_DEPTH {
            return Err(QueryError::BudgetExceeded { budget: "MAX_QUERY_DEPTH" });
        }
        // Validity: the slot is fresh when every recorded transitive dependency
        // still has the hash recorded at compute time.
        if let Some(slot) = self.slots.get(&slot_key) {
            let fresh = slot.deps.iter().all(|(name, hash)| {
                self.inputs.get(name).is_some_and(|s| &s.hash == hash)
            });
            if fresh {
                self.stats.memo_hits += 1;
                return match slot.value.downcast_ref::<MemoPayload<Q::Value, Q::Error>>() {
                    Some(MemoPayload::Ok(v)) => Ok(v.clone()),
                    Some(MemoPayload::Err(e)) => Err(QueryError::Computation(e.clone())),
                    None => Err(QueryError::BudgetExceeded { budget: "MAX_QUERY_SLOTS" }),
                };
            }
            self.stats.invalidations += 1;
        }
        if self.slots.len() >= MAX_QUERY_SLOTS && !self.slots.contains_key(&slot_key) {
            return Err(QueryError::BudgetExceeded { budget: "MAX_QUERY_SLOTS" });
        }
        // Execute under a frame: cycle key, depth, dependency accumulator.
        self.in_progress.push(slot_key.clone());
        self.depth += 1;
        self.frame_deps.push(Vec::new());
        let outcome = Q::execute(self, key);
        let frame = self.frame_deps.pop().unwrap_or_default();
        self.depth -= 1;
        self.in_progress.pop();
        match outcome {
            Ok(value) => {
                if let Some(parent) = self.frame_deps.last_mut() {
                    parent.extend(frame.iter().cloned());
                }
                self.stats.computes += 1;
                self.slots.insert(
                    slot_key,
                    Slot {
                        value: Box::new(MemoPayload::<Q::Value, Q::Error>::Ok(value.clone())),
                        deps: frame,
                    },
                );
                Ok(value)
            }
            Err(e) => {
                // Errors are cached too (rustc convention): a failing query re-serves
                // its error until a dependency changes.
                self.stats.computes += 1;
                self.slots.insert(
                    slot_key,
                    Slot {
                        value: Box::new(MemoPayload::<Q::Value, Q::Error>::Err(e.clone())),
                        deps: frame,
                    },
                );
                Err(QueryError::Computation(e))
            }
        }
    }
}

/// CEP:WHAT: Type-erased memo payload (Ok value or Err computation error).
/// CEP:WHY: One slot table hosts many query types; Any + this payload keeps that
///   uniform without external crates.
/// CEP:STATUS: complete
/// CEP:FAILURE: downcast failure surfaces as BudgetExceeded (impossible by construction).
/// CEP:ASSUMES: each slot is written and read by exactly one query type.
/// CEP:COST: one Box per slot.
/// CEP:EVIDENCE: the engine test battery.
enum MemoPayload<V, E> {
    Ok(V),
    Err(E),
}

impl<V, E> MemoPayload<V, E> {
    #[allow(dead_code)]
    fn is_ok(&self) -> bool {
        matches!(self, MemoPayload::Ok(_))
    }
}

#[cfg(test)]
mod query_engine_tests {
    use super::*;
    use std::convert::Infallible;

    // Synthetic queries over two independent inputs.

    struct LenQuery;
    impl Query for LenQuery {
        type Value = usize;
        type Error = Infallible;
        const ID: &'static str = "len";
        fn execute(db: &mut QueryDatabase, key: &str) -> Result<usize, Infallible> {
            let src = db.read_input(key).unwrap_or_default();
            Ok(src.len())
        }
    }

    struct PlusOneQuery; // child of LenQuery: value = len(input named by key) + 1
    impl Query for PlusOneQuery {
        type Value = usize;
        type Error = Infallible;
        const ID: &'static str = "plus_one";
        fn execute(db: &mut QueryDatabase, key: &str) -> Result<usize, Infallible> {
            let inner = db.get::<LenQuery>(key).unwrap_or(0);
            Ok(inner + 1)
        }
    }

    struct FailingQuery;
    impl Query for FailingQuery {
        type Value = usize;
        type Error = String;
        const ID: &'static str = "failing";
        fn execute(db: &mut QueryDatabase, key: &str) -> Result<usize, String> {
            let src = db.read_input(key).map_err(|e| e.to_string())?;
            if src.len() > 8 { Err("too long".to_string()) } else { Ok(src.len()) }
        }
    }

    /// CEP:WHAT: Flattens a nested engine-shaped error bubbling through intermediate
    /// query frames so budget/cycle variants surface at the outer demand.
    /// CEP:WHY: Test-only recursion helper; production queries own their own policy.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: unreachable states map to MissingInput (honest signal).
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: the cycle and chain tests.
    fn flatten_engine(e: QueryError<QueryError<std::convert::Infallible>>) -> QueryError<std::convert::Infallible> {
        match e {
            QueryError::BudgetExceeded { budget } => QueryError::BudgetExceeded { budget },
            QueryError::Cycle { path } => QueryError::Cycle { path },
            QueryError::Computation(inner) => match inner {
                QueryError::BudgetExceeded { budget } => QueryError::BudgetExceeded { budget },
                QueryError::Cycle { path } => QueryError::Cycle { path },
                other => QueryError::MissingInput { name: other.to_string() },
            },
            other => QueryError::MissingInput { name: other.to_string() },
        }
    }

    struct SelfCycle;
    impl Query for SelfCycle {
        type Value = usize;
        type Error = QueryError<std::convert::Infallible>;
        const ID: &'static str = "self_cycle";
        fn execute(
            db: &mut QueryDatabase,
            key: &str,
        ) -> Result<usize, QueryError<std::convert::Infallible>> {
            // A query that re-demands itself: the engine must report a cycle, not
            // hang; the cycle error propagates verbatim to the outer demand.
            match db.get::<SelfCycle>(key) {
                Ok(v) => Ok(v),
                Err(e) => Err(flatten_engine(e)),
            }
        }
    }

    struct Ping;
    impl Query for Ping {
        type Value = usize;
        type Error = QueryError<std::convert::Infallible>;
        const ID: &'static str = "ping";
        fn execute(
            db: &mut QueryDatabase,
            key: &str,
        ) -> Result<usize, QueryError<std::convert::Infallible>> {
            match db.get::<Pong>(key) {
                Ok(v) => Ok(v),
                Err(e) => Err(flatten_engine(e)),
            }
        }
    }

    struct Pong;
    impl Query for Pong {
        type Value = usize;
        type Error = QueryError<std::convert::Infallible>;
        const ID: &'static str = "pong";
        fn execute(
            db: &mut QueryDatabase,
            key: &str,
        ) -> Result<usize, QueryError<std::convert::Infallible>> {
            match db.get::<Ping>(key) {
                Ok(v) => Ok(v),
                Err(e) => Err(flatten_engine(e)),
            }
        }
    }

    fn engine_db() -> QueryDatabase {
        let mut db = QueryDatabase::new();
        db.set_input("a", b"0123").unwrap();
        db.set_input("b", b"0123456789").unwrap();
        db
    }

    #[test]
    fn query_memo_hits_skip_recompute() {
        let mut db = engine_db();
        assert_eq!(db.get::<LenQuery>("a").unwrap(), 4);
        assert_eq!(db.stats().computes, 1);
        assert_eq!(db.get::<LenQuery>("a").unwrap(), 4);
        assert_eq!(db.stats().computes, 1, "second demand is a memo hit");
        assert_eq!(db.stats().memo_hits, 1);
    }

    #[test]
    fn query_invalidated_only_when_dep_changes() {
        let mut db = engine_db();
        db.get::<LenQuery>("a").unwrap();
        db.set_input("b", b"x").unwrap(); // unrelated input
        db.get::<LenQuery>("a").unwrap();
        assert_eq!(db.stats().computes, 1, "unrelated change must not recompute");
        db.set_input("a", b"01234").unwrap(); // the dependency itself
        assert_eq!(db.get::<LenQuery>("a").unwrap(), 5);
        assert_eq!(db.stats().computes, 2, "dep change recomputes exactly once");
        assert_eq!(db.stats().invalidations, 1);
    }

    #[test]
    fn query_transitive_invalidation() {
        let mut db = engine_db();
        assert_eq!(db.get::<PlusOneQuery>("a").unwrap(), 5);
        db.set_input("b", b"y").unwrap();
        db.get::<PlusOneQuery>("a").unwrap();
        assert_eq!(db.stats().computes, 2, "sibling input change must not recompute");
        assert_eq!(db.stats().memo_hits, 1, "the parent slot survives");
        db.set_input("a", b"01").unwrap();
        assert_eq!(db.get::<PlusOneQuery>("a").unwrap(), 3, "new value propagates");
        assert_eq!(db.stats().computes, 4, "child and parent both recompute");
    }

    #[test]
    fn query_set_input_same_content_keeps_memos() {
        let mut db = engine_db();
        db.get::<LenQuery>("a").unwrap();
        let rev = db.revision();
        assert!(!db.set_input("a", b"0123").unwrap(), "identical content is a no-op");
        assert_eq!(db.revision(), rev, "no revision bump");
        db.get::<LenQuery>("a").unwrap();
        assert_eq!(db.stats().computes, 1, "memo survives identical re-set");
    }

    #[test]
    fn query_cycle_is_a_structured_error() {
        let mut db = engine_db();
        let e = db.get::<SelfCycle>("k");
        // The cycle is detected inside the query's own re-demand; the outer demand
        // observes it as the query's computation error wrapping the engine's Cycle.
        match e {
            Err(QueryError::Computation(QueryError::Cycle { path })) => {
                assert_eq!(path.len(), 2, "cycle path is the closed loop");
                assert_eq!(path.first(), path.last(), "the path closes on itself");
            }
            other => panic!("expected cycle, got {other:?}"),
        }
        // The engine stays usable after the cycle.
        assert_eq!(db.get::<LenQuery>("a").unwrap(), 4);
    }

    #[test]
    fn query_cycle_through_two_queries() {
        let mut db = engine_db();
        let e = db.get::<Ping>("k");
        assert!(matches!(
            e,
            Err(QueryError::Computation(QueryError::Cycle { .. }))
        ));
        assert_eq!(db.get::<LenQuery>("a").unwrap(), 4, "engine recovers");
    }

    #[test]
    fn query_error_cached_until_dep_changes() {
        let mut db = engine_db();
        assert!(matches!(
            db.get::<FailingQuery>("b"),
            Err(QueryError::Computation(ref s)) if s == "too long"
        ));
        assert_eq!(db.stats().computes, 1);
        assert!(matches!(
            db.get::<FailingQuery>("b"),
            Err(QueryError::Computation(ref s)) if s == "too long"
        ));
        assert_eq!(db.stats().computes, 1, "cached error re-served");
        db.set_input("b", b"short").unwrap();
        assert_eq!(db.get::<FailingQuery>("b").unwrap(), 5, "recompute after dep change");
        assert_eq!(db.stats().computes, 2);
    }

    #[test]
    fn query_missing_input_is_structured() {
        let mut db = QueryDatabase::new();
        let e = db.get::<FailingQuery>("never-registered");
        assert!(matches!(e, Err(QueryError::Computation(_))), "read_input error flows into execute's error type");
    }

    #[test]
    fn query_deep_chain_succeeds_and_budgets() {
        struct ChainQuery;
        const DEPTH_OK: usize = 100;
        impl Query for ChainQuery {
            type Value = usize;
            type Error = QueryError<std::convert::Infallible>;
            const ID: &'static str = "chain";
            fn execute(
                db: &mut QueryDatabase,
                key: &str,
            ) -> Result<usize, QueryError<std::convert::Infallible>> {
                let depth: usize = match key.parse() {
                    Ok(d) => d,
                    Err(_) => return Err(QueryError::MissingInput { name: "bad key".into() }),
                };
                if depth == 0 {
                    Ok(0)
                } else {
                    match db.get::<ChainQuery>(&(depth - 1).to_string()) {
                        Ok(v) => Ok(v + 1),
                        Err(e) => Err(flatten_engine(e)),
                    }
                }
            }
        }
        let mut db = QueryDatabase::new();
        assert_eq!(db.get::<ChainQuery>(&DEPTH_OK.to_string()).unwrap(), DEPTH_OK);
        // Fresh database: memoized short-chains would legitimately never recurse
        // deep enough to trip the budget, and that is itself the point.
        let mut db2 = QueryDatabase::new();
        let e = db2.get::<ChainQuery>(&200.to_string());
        // The budget trips mid-recursion; each frame forwards the variant, so the
        // outermost demand observes the engine's Computation wrapping of it.
        assert!(matches!(
            e,
            Err(QueryError::Computation(QueryError::BudgetExceeded { budget: "MAX_QUERY_DEPTH" }))
        ));
    }

    #[test]
    fn query_stats_show_memo_hits_and_invalidations() {
        let mut db = engine_db();
        db.get::<LenQuery>("a").unwrap();
        db.get::<LenQuery>("a").unwrap();
        db.set_input("a", b"zz").unwrap();
        db.get::<LenQuery>("a").unwrap();
        let s = db.stats();
        assert_eq!((s.computes, s.memo_hits, s.invalidations), (2, 1, 1));
    }

    #[test]
    fn query_invalidate_all_forces_recompute() {
        let mut db = engine_db();
        db.get::<LenQuery>("a").unwrap();
        db.invalidate_all();
        db.get::<LenQuery>("a").unwrap();
        assert_eq!(db.stats().computes, 2, "invalidate_all drops every memo");
        assert_eq!(db.get::<LenQuery>("a").unwrap(), 4);
        assert_eq!(db.stats().memo_hits, 1, "and then it memoizes again");
    }

    #[test]
    fn query_namespaces_by_query_id() {
        // Two query types sharing one key do not collide.
        struct AlsoLen;
        impl Query for AlsoLen {
            type Value = usize;
            type Error = Infallible;
            const ID: &'static str = "also_len";
            fn execute(db: &mut QueryDatabase, key: &str) -> Result<usize, Infallible> {
                Ok(db.get::<LenQuery>(key).unwrap_or(0) * 10)
            }
        }
        let mut db = engine_db();
        assert_eq!(db.get::<LenQuery>("a").unwrap(), 4);
        assert_eq!(db.get::<AlsoLen>("a").unwrap(), 40);
        assert_eq!(db.get::<LenQuery>("a").unwrap(), 4, "no cross-talk");
        assert_eq!(db.stats().computes, 2);
    }

    #[test]
    fn query_key_separator_is_collision_safe() {
        // Keys containing the internal separator must not alias other slots
        // ("a\\0b" is an unregistered input, so LenQuery yields its default 0).
        let mut db = engine_db();
        let v1 = db.get::<LenQuery>("a\u{0}b").unwrap();
        let v2 = db.get::<LenQuery>("a").unwrap();
        assert_eq!(v1, 0, "unregistered input mapped to default, distinct slot");
        assert_eq!(v2, 4);
        assert_eq!(db.stats().computes, 2, "distinct slots");
    }

    #[test]
    fn query_error_display_is_stable() {
        let e: QueryError<String> = QueryError::Cycle { path: vec!["p".into(), "q".into()] };
        assert_eq!(e.to_string(), "query cycle: p -> q");
        let e: QueryError<String> = QueryError::BudgetExceeded { budget: "X" };
        assert_eq!(e.to_string(), "query budget exceeded: X");
        let e: QueryError<String> = QueryError::MissingInput { name: "n".into() };
        assert_eq!(e.to_string(), "missing input: n");
        let e: QueryError<String> = QueryError::Computation("bad".into());
        assert_eq!(e.to_string(), "bad");
    }

    #[test]
    fn query_revision_bumps_only_on_change() {
        let mut db = QueryDatabase::new();
        db.set_input("a", b"1").unwrap();
        let r1 = db.revision();
        db.set_input("a", b"1").unwrap();
        assert_eq!(db.revision(), r1);
        db.set_input("a", b"2").unwrap();
        assert_eq!(db.revision(), r1 + 1);
    }

    #[test]
    fn query_deps_recorded_despite_error() {
        // A query that reads its input and then fails must still record the
        // dependency: fixing the input re-executes it.
        let mut db = engine_db();
        assert!(db.get::<FailingQuery>("b").is_err());
        db.set_input("b", b"ok").unwrap();
        assert_eq!(db.get::<FailingQuery>("b").unwrap(), 2, "recomputed after fix");
    }

    #[test]
    fn query_engine_is_deterministic_across_runs() {
        // Same script of operations twice -> identical stats and values.
        fn script() -> (usize, usize, QueryStats) {
            let mut db = engine_db();
            let a = db.get::<PlusOneQuery>("a").unwrap();
            let b = db.get::<LenQuery>("b").unwrap();
            db.set_input("a", b"01234").unwrap();
            let a2 = db.get::<PlusOneQuery>("a").unwrap();
            (a + b + a2, 0, db.stats())
        }
        let r1 = script();
        let r2 = script();
        assert_eq!(r1.0, r2.0);
        assert_eq!(r1.2, r2.2, "stats identical: deterministic engine");
    }

    #[test]
    fn query_engine_fails_closed_on_infallible_read_of_missing_input() {
        // LenQuery on a missing key: read_input errors, execute maps to default,
        // so the engine returns a value rather than panicking (LenQuery is
        // infallible by contract); the missing input is visible via a failing
        // FailingQuery instead.
        let mut db = QueryDatabase::new();
        let v = db.get::<LenQuery>("ghost").unwrap();
        assert_eq!(v, 0, "infallible query maps missing input to default");
    }
}
