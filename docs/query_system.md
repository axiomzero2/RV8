# Demand-Driven Query System

## Abstract
`rv8_driver::query` implements the memoized, demand-driven query database: queries execute on demand, memoize values and errors, record their input dependencies at read time, and re-execute only when a transitive dependency's BLAKE3 content hash changed. `rv8_driver::queries` defines the production query set (FunctionCountQuery, FunctionObjectQuery, ProgramQuery) and `rv8_driver::pipeline::compile` demands ProgramQuery.

## Formal Definitions
```rust
pub trait Query: 'static {
    type Value: Clone + 'static;
    type Error: Clone + 'static;
    const ID: &'static str;
    fn execute(db: &mut QueryDatabase, key: &str) -> Result<Self::Value, Self::Error>;
}
pub enum QueryError<E> { Cycle { path: Vec<String> }, BudgetExceeded { budget: &'static str },
                         MissingInput { name: String }, Computation(E) }
impl QueryDatabase {
    pub fn new() -> QueryDatabase;
    pub fn set_input(&mut self, name: &str, bytes: &[u8]) -> Result<bool, QueryError<Infallible>>;
    pub fn read_input(&mut self, name: &str) -> Result<Vec<u8>, QueryError<Infallible>>;
    pub fn get<Q: Query>(&mut self, key: &str) -> Result<Q::Value, QueryError<Q::Error>>;
    pub fn invalidate_all(&mut self);
    pub fn stats(&self) -> QueryStats;   // computes, memo_hits, invalidations
    pub fn revision(&self) -> u64;
}
pub const MAX_QUERY_DEPTH: usize = 128;
pub const MAX_QUERY_SLOTS: usize = 1 << 20;
pub const MAX_QUERY_INPUTS: usize = 1 << 20;
```
A memo slot stores the erased payload (value or cached error) plus the transitive dependency set `deps: Vec<(input name, BLAKE3 hash)>` recorded at compute time.

## Algorithm
1. `get<Q>(key)`: reject cycles via the in-progress stack (the cycle path is returned, not unwound silently); reject past the depth budget; verify the slot against its dependency hashes (all unchanged means a hit, counters updated); otherwise execute under a fresh frame.
2. `read_input` registers `(name, hash)` in the executing frame, so an execute cannot forget a dependency.
3. When a child query completes, its slot's dependency set merges into the parent frame: parent deps are the transitive union.
4. Errors cache exactly like values and re-serve until a dependency changes.
5. `set_input` hashes content with BLAKE3; identical content is a no-op (no revision bump, no invalidation).

Memo hit cost is O(deps); recompute is the query's own cost. Assumptions: execute is deterministic (STANDARDS 2.2) and pure; single-threaded ownership.

## Invariants
1. Unchanged dependency hashes imply the cached value is returned without re-execution (pinned per query and per pipeline).
2. A dependency change re-executes exactly the queries that transitively read it; sibling-input changes do not invalidate.
3. Query cycles produce `QueryError::Cycle` with a closed path; the engine remains usable afterwards.
4. Failing queries cache their error until a dependency changes (recompute on fix).
5. Depth over MAX_QUERY_DEPTH and unbounded key/input spaces are structured BudgetExceeded failures, never stack exhaustion or unbounded memory.
6. Identical operation scripts produce identical stats (determinism test).

## Integration Points
Depends on: `rv8_core::blake3` (input hashes), `rv8_frontend`, `rv8_ir`, `rv8_middle`, `rv8_backend`, `rv8_object` (query bodies). Depended on by: `pipeline::compile(session, src) -> Result<CompiledProgram, DriverError>`, which registers the `source` input and demands `ProgramQuery("")`; the Run-mode main-function requirement stays in the pipeline where the session is known.

## Known Limitations
The engine is single-threaded (crate-DAG parallel scheduling is future work). Each FunctionObjectQuery re-parses the whole source because the frontend is not yet function-incremental; every function query therefore depends on the entire source, and finer invalidation granularity arrives with the frontend module dependency graph (design.md frontend incrementality). `invalidate_all` drops all memos (compiler-version bumps) but keeps inputs.
