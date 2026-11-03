// CEP:FILE: crates/rv8_driver/src/queries.rs
// CEP:WHAT: The concrete driver query set: per-function object code, per-program
//   assembly, and the function count, all demand-driven over the registered source
//   input through the QueryDatabase engine.
// CEP:WHY: Design.md's "Memoized Query System" requires the pipeline to run as
//   queries, not a rigid sequence; these are the production queries the driver
//   executes (RV8-134).
// CEP:CLASS: HPC-1
// CEP:STATUS: complete
// CEP:OWNER: axiomzero2
// CEP:TICKET: RV8-134
// CEP:FAILURE: DriverError surfaces as QueryError::Computation; engine errors
//   (cycle/budget/missing input) map onto DriverError::Stage in pipeline.rs.
// CEP:ASSUMES: the frontend parses the whole source per query execution (the module
//   dependency graph of design.md's frontend incrementality is future work); execute
//   is deterministic per STANDARDS 2.2.
// CEP:COST: each function query re-parses the source (O(source)); memo hits are free.
// CEP:EVIDENCE: unit tests here and in query.rs; pipeline.rs routes through this.
use crate::error::DriverError;
use crate::query::{Query, QueryDatabase, QueryError, StageHashes};
use rv8_object::model::{ObjectFile, RelocKind as ObjRelocKind};

/// CEP:WHAT: The input name every driver query reads the source text from.
/// CEP:WHY: One canonical input name keeps dependency recording uniform.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: pipeline.rs registers exactly this name.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: pipeline::compile.
pub const SOURCE_INPUT: &str = "source";

/// CEP:WHAT: One compiled function: code bytes, relocations, symbol facts, and the
///   per-level stage hashes.
/// CEP:WHY: The memoized unit of codegen; the program query assembles these.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: owns the code bytes.
/// CEP:EVIDENCE: function_object_query_is_memoized_and_invalidated.
#[derive(Clone, Debug)]
pub struct FunctionObject {
    pub function: String,
    pub symbol: String,
    pub section: String,
    pub code: Vec<u8>,
    pub relocs: Vec<(u64, rv8_backend::encode::RelocKind, String, i64)>,
    pub code_len: u64,
    pub hashes: StageHashes,
}

/// CEP:WHAT: Compiles one function (by ordinal key) through the normative pipeline:
///   flatten, borrowck, SoN passes, linearize, HetIR passes, LIR, backend.
/// CEP:WHY: Per-function memoization is the granularity the pipeline exposes today;
///   the stage order is the normative pipeline (pipeline.rs documents it).
/// CEP:STATUS: complete
/// CEP:FAILURE: QueryError::Computation(DriverError::Stage) naming the failing stage.
/// CEP:ASSUMES: key parses as usize; the source input is registered.
/// CEP:COST: O(source parse + per-function passes).
/// CEP:EVIDENCE: function_object_query_is_memoized_and_invalidated.
pub struct FunctionObjectQuery;
impl Query for FunctionObjectQuery {
    type Value = FunctionObject;
    type Error = DriverError;
    const ID: &'static str = "function_object";

    fn execute(db: &mut QueryDatabase, key: &str) -> Result<FunctionObject, DriverError> {
        let ordinal: usize = key
            .parse()
            .map_err(|_| DriverError::Stage { stage: "query", message: format!("bad function ordinal key {key:?}") })?;
        let src = db
            .read_input(SOURCE_INPUT)
            .map_err(|e| DriverError::Stage { stage: "query", message: e.to_string() })?;
        let src = String::from_utf8(src).map_err(|e| DriverError::Stage {
            stage: "query",
            message: format!("source is not UTF-8: {e}"),
        })?;
        let hirs = rv8_frontend::compile_source(&src)
            .map_err(|e| DriverError::Stage { stage: "frontend", message: e.to_string() })?;
        let hir = hirs.get(ordinal).ok_or_else(|| DriverError::Stage {
            stage: "query",
            message: format!("function ordinal {ordinal} out of range ({})", hirs.len()),
        })?;
        compile_one(hir, ordinal)
    }
}

/// CEP:WHAT: The function count of the source (the program query's fan-out).
/// CEP:WHY: The program query must know how many function queries to demand.
/// CEP:STATUS: complete
/// CEP:FAILURE: QueryError::Computation(DriverError::Stage).
/// CEP:ASSUMES: the source input is registered.
/// CEP:COST: O(source parse).
/// CEP:EVIDENCE: program_query_assembles_the_object.
pub struct FunctionCountQuery;
impl Query for FunctionCountQuery {
    type Value = usize;
    type Error = DriverError;
    const ID: &'static str = "function_count";

    fn execute(db: &mut QueryDatabase, _key: &str) -> Result<usize, DriverError> {
        let src = db
            .read_input(SOURCE_INPUT)
            .map_err(|e| DriverError::Stage { stage: "query", message: e.to_string() })?;
        let src = String::from_utf8(src).map_err(|e| DriverError::Stage {
            stage: "query",
            message: format!("source is not UTF-8: {e}"),
        })?;
        rv8_frontend::compile_source(&src)
            .map(|hirs| hirs.len())
            .map_err(|e| DriverError::Stage { stage: "frontend", message: e.to_string() })
    }
}

/// CEP:WHAT: The whole-program query: assembles the packed object, the stage log, and
///   the entry symbol from the per-function queries.
/// CEP:WHY: The driver entry point compiles by demanding this single query.
/// CEP:STATUS: complete
/// CEP:FAILURE: QueryError::Computation(DriverError) from children.
/// CEP:ASSUMES: key is the empty string (one program per database).
/// CEP:COST: O(functions x per-function cost).
/// CEP:EVIDENCE: program_query_assembles_the_object.
pub struct ProgramQuery;
impl Query for ProgramQuery {
    type Value = crate::pipeline::CompiledProgram;
    type Error = DriverError;
    const ID: &'static str = "program";

    fn execute(db: &mut QueryDatabase, _key: &str) -> Result<crate::pipeline::CompiledProgram, DriverError> {
        let count = db
            .get::<FunctionCountQuery>("")
            .map_err(|e| map_engine(e, "function_count"))?;
        let mut program = ObjectFile::new("rv8");
        let mut stages = crate::query::StageLog::new();
        let mut entry_symbol: Option<String> = None;
        for ordinal in 0..count {
            let fo = db
                .get::<FunctionObjectQuery>(&ordinal.to_string())
                .map_err(|e| map_engine(e, "function_object"))?;
            if fo.function == "main" {
                entry_symbol = Some(fo.function.clone());
            }
            let sec = program
                .add_text_section(&fo.section, 16, fo.code.clone())
                .map_err(|e| DriverError::Stage { stage: "object", message: e.to_string() })?;
            for (offset, kind, symbol, addend) in &fo.relocs {
                let k = match kind {
                    rv8_backend::encode::RelocKind::Abs64 => ObjRelocKind::Abs64,
                    rv8_backend::encode::RelocKind::PcRel32 => ObjRelocKind::PcRel32,
                };
                program
                    .add_reloc(sec, *offset, k, symbol, *addend)
                    .map_err(|e| DriverError::Stage { stage: "object", message: e.to_string() })?;
            }
            program
                .add_symbol(&fo.symbol, true, true, Some(sec), 0, fo.code_len)
                .map_err(|e| DriverError::Stage { stage: "object", message: e.to_string() })?;
            program
                .add_symbol(&fo.function, true, true, Some(sec), 0, fo.code_len)
                .map_err(|e| DriverError::Stage { stage: "object", message: e.to_string() })?;
            stages.record(fo.hashes);
        }
        Ok(crate::pipeline::CompiledProgram { object: program, entry_symbol, stages })
    }
}

/// CEP:WHAT: Maps engine failures onto DriverError::Stage with the query id named.
/// CEP:WHY: The CLI reports one error taxonomy.
/// CEP:STATUS: complete
/// CEP:FAILURE: this is the mapping.
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: pipeline tests exercise the mapping via compile.
fn map_engine(e: QueryError<DriverError>, query: &'static str) -> DriverError {
    match e {
        QueryError::Computation(d) => d,
        other => DriverError::Stage { stage: "query", message: format!("{query}: {other}") },
    }
}

/// CEP:WHAT: The shared per-function stage chain (flatten through backend), extracted
///   from pipeline::compile so the query and the legacy entry share one path.
/// CEP:WHY: Two entry points compiling differently would violate the one-pipeline law.
/// CEP:STATUS: complete
/// CEP:FAILURE: DriverError::Stage naming the failing stage.
/// CEP:ASSUMES: hir passes the level-0 invariants (frontend guarantees).
/// CEP:COST: the pipeline's own cost.
/// CEP:EVIDENCE: end_to_end semantics suites (26+22 tests) still route through here.
fn compile_one(hir: &rv8_ir::hir::HirFunction, ordinal: usize) -> Result<FunctionObject, DriverError> {
    let fn_name = hir.name.clone();
    let mut sea = rv8_ir::lowering::hir_to_son(hir)
        .map_err(|e| DriverError::Stage { stage: "hir_to_son", message: e.to_string() })?;
    let son_hash = sea.canonical_hash();
    let borrow = rv8_middle::borrowck::check(&sea)
        .map_err(|e| DriverError::Stage { stage: "borrowck", message: e.to_string() })?;
    if !borrow.conflicts.is_empty() {
        return Err(DriverError::Stage {
            stage: "borrowck",
            message: format!("{:?}", borrow.conflicts),
        });
    }
    let _summary_son = rv8_middle::pipeline::optimize_son(&mut sea)
        .map_err(|e| DriverError::Stage { stage: "optimize_son", message: e.to_string() })?;
    // Partial escape analysis (standard 5.6) runs after the SoN optimization pipeline
    // and before lowering: scalar replacement and cold-path materialization rewrite the
    // graph in place; the stage hash below therefore covers the post-PEA content.
    let _pea = rv8_middle::pea::run_pea(&mut sea)
        .map_err(|e| DriverError::Stage { stage: "pea", message: e.to_string() })?;
    let son_opt_hash = sea.canonical_hash();
    let mut het = rv8_ir::lower_het::son_to_hetir(&sea)
        .map_err(|e| DriverError::Stage { stage: "son_to_hetir", message: e.to_string() })?;
    let het_hash = het.canonical_hash();
    let _summary_het = rv8_middle::pipeline::optimize_hetir(&mut het)
        .map_err(|e| DriverError::Stage { stage: "optimize_hetir", message: e.to_string() })?;
    let het_opt_hash = het.canonical_hash();
    let lir = rv8_ir::lower_het::hetir_to_lir(&het, &fn_name)
        .map_err(|e| DriverError::Stage { stage: "hetir_to_lir", message: e.to_string() })?;
    let symbol = rv8_backend::encode::call_symbol_name(ordinal as i64);
    let out = rv8_backend::compile_lir(&lir, &symbol)
        .map_err(|e| DriverError::Stage { stage: "backend", message: e.to_string() })?;
    let mir_hash = rv8_core::hash::fxhash64(&out.code);
    Ok(FunctionObject {
        section: format!(".text.{symbol}"),
        function: fn_name.clone(),
        relocs: out
            .relocs
            .iter()
            .map(|r| (r.offset, r.kind, r.symbol.clone(), r.addend))
            .collect(),
        code_len: out.code.len() as u64,
        code: out.code,
        symbol,
        hashes: StageHashes {
            function: fn_name,
            son: son_hash,
            son_opt: son_opt_hash,
            het: het_hash,
            het_opt: het_opt_hash,
            mir: mir_hash,
        },
    })
}

// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::QueryError;

    const SRC: &str = "fn main() -> i32 { let a = 6; let b = 7; return a + b; }";
    const SRC2: &str = "fn main() -> i32 { let a = 5; let b = 8; return a + b; }";

    fn db_with(src: &str) -> QueryDatabase {
        let mut db = QueryDatabase::new();
        db.set_input(SOURCE_INPUT, src.as_bytes()).unwrap();
        db
    }

    /// CEP:WHAT: The program query assembles the fixture object and the second demand
    /// is a pure memo hit (zero recompute).
    /// CEP:WHY: The core incrementality contract at the pipeline level.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn program_query_assembles_the_object() {
        let mut db = db_with(SRC);
        let p1 = db.get::<ProgramQuery>("").unwrap();
        assert!(p1.entry_symbol.as_deref() == Some("main"));
        let stats_after_first = db.stats();
        assert_eq!(stats_after_first.computes, 3, "count + one function + program");
        let p2 = db.get::<ProgramQuery>("").unwrap();
        assert!(p1.stages.equals(&p2.stages), "stage hashes identical on re-demand");
        assert_eq!(p1.entry_symbol, p2.entry_symbol);
        assert_eq!(db.stats().computes, stats_after_first.computes, "no recompute");
        assert_eq!(db.stats().memo_hits, 1, "exactly the program slot hits; children are not re-demanded");
    }

    /// CEP:WHAT: The function query memoizes; changing the source invalidates exactly
    /// once and recomputes.
    /// CEP:WHY: Content-addressed invalidation at the function granularity.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn function_object_query_is_memoized_and_invalidated() {
        let mut db = db_with(SRC);
        let a1 = db.get::<FunctionObjectQuery>("0").unwrap();
        let a2 = db.get::<FunctionObjectQuery>("0").unwrap();
        assert_eq!(a1.code, a2.code);
        assert_eq!(db.stats().computes, 1);
        db.set_input(SOURCE_INPUT, SRC2.as_bytes()).unwrap();
        let a3 = db.get::<FunctionObjectQuery>("0").unwrap();
        assert_eq!(db.stats().computes, 2);
        assert_eq!(db.stats().invalidations, 1);
        assert_eq!(a3.function, "main");
    }

    /// CEP:WHAT: Out-of-range ordinals and malformed keys are structured failures,
    /// cached until the source changes (rustc error-caching convention).
    /// CEP:WHY: "Do not just pass anything": bad keys must not silently pass.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn function_query_rejects_bad_keys_and_caches_errors() {
        let mut db = db_with(SRC);
        let e1 = db.get::<FunctionObjectQuery>("not-a-number").unwrap_err();
        assert!(matches!(e1, QueryError::Computation(_)));
        let e2 = db.get::<FunctionObjectQuery>("not-a-number").unwrap_err();
        assert!(matches!(e2, QueryError::Computation(_)));
        assert_eq!(db.stats().computes, 1, "error cached, not recomputed");
        let e3 = db.get::<FunctionObjectQuery>("9").unwrap_err();
        assert!(matches!(e3, QueryError::Computation(_)), "ordinal out of range");
        let ok = db.get::<FunctionObjectQuery>("0").unwrap();
        assert_eq!(ok.function, "main");
    }

    /// CEP:WHAT: A frontend failure (unparseable source) surfaces through the program
    /// query as a structured stage error, not a panic, and stays cached.
    /// CEP:WHY: Fail-closed end to end.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn frontend_failures_are_structured_and_cached() {
        let mut db = db_with("fn main() -> i32 { let = ; }");
        let e = db.get::<ProgramQuery>("").unwrap_err();
        assert!(matches!(e, QueryError::Computation(DriverError::Stage { .. })));
        let e2 = db.get::<ProgramQuery>("").unwrap_err();
        assert!(matches!(e2, QueryError::Computation(_)));
        assert_eq!(db.stats().computes, 2, "count + program queries computed once each");
    }
}
