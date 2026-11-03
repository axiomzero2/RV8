// CEP:FILE: crates/rv8_driver/src/pipeline.rs
// CEP:WHAT: The full compilation pipeline: source text through frontend, five IR levels,
//   middle-end passes with borrow checking, backend selection and allocation, x86-64
//   encoding, object packing, static linking, and execution.
// CEP:WHY: One auditable function per stage boundary (spec section 7 integration points);
//   the driver is the only place every crate meets, so ordering and verification policy
//   are visible in one file.
// CEP:CLASS: HPC-1
// CEP:STATUS: complete
// CEP:FAILURE: every stage maps its error into DriverError::Stage with the stage name.
// CEP:ASSUMES: functions call each other by declaration ordinal through call_symbol_name;
//   the entry function is named main for run mode.
// CEP:COST: linear in program size per stage; no cross-function hidden passes.
// CEP:EVIDENCE: tests pipeline_runs_fixture_exit_13, pipeline_is_deterministic, and
//   pipeline_runs_loop_program in this module.

use crate::error::DriverError;
use crate::session::{EmitMode, Session};
use rv8_object::model::ObjectFile;


/// CEP:WHAT: The packed compilation output: one relocatable object holding every function
///   plus the stage-hash log gathered on the way.
/// CEP:WHY: Run mode needs the object for linking; obj mode serializes it directly; the
///   stage log is the determinism evidence trail.
/// CEP:STATUS: complete
/// CEP:FAILURE: construction only through compile.
/// CEP:ASSUMES: none
/// CEP:COST: owns the object bytes.
/// CEP:EVIDENCE: tests in this module.
#[derive(Clone, Debug)]
pub struct CompiledProgram {
    /// One object containing one `.text.fn<i>` section per function.
    pub object: ObjectFile,
    /// Real name of the entry function when one is named main.
    pub entry_symbol: Option<String>,
    /// Per-level canonical hashes recorded during compilation.
    pub stages: crate::query::StageLog,
}

/// CEP:WHAT: Compiles source text into a packed object through the whole pipeline,
///   demand-driven through the QueryDatabase (ProgramQuery over per-function
///   queries; RV8-134): the stage order inside each query is the normative pipeline.
/// CEP:WHY: The single entry point main.rs calls; design.md fixes the query-driven
///   architecture and STANDARDS 5.10 the content-addressed invalidation.
/// CEP:STATUS: complete
/// CEP:FAILURE: DriverError::Stage naming the failing stage on any rejection.
/// CEP:ASSUMES: src is UTF-8 Rust subset text.
/// CEP:COST: first demand compiles; re-demands on unchanged source are memo hits.
/// CEP:EVIDENCE: test pipeline_runs_fixture_exit_13 and the query battery.
pub fn compile(session: &Session, src: &str) -> Result<CompiledProgram, DriverError> {
    let mut db = crate::query::QueryDatabase::new();
    db.set_input(crate::queries::SOURCE_INPUT, src.as_bytes())
        .map_err(|e| DriverError::Stage { stage: "query", message: e.to_string() })?;
    let program = db
        .get::<crate::queries::ProgramQuery>("")
        .map_err(|e| match e {
            crate::query::QueryError::Computation(d) => d,
            other => DriverError::Stage { stage: "query", message: other.to_string() },
        })?;
    if session.emit == EmitMode::Run && program.entry_symbol.is_none() {
        return Err(DriverError::Stage {
            stage: "link",
            message: "no function named main: run mode requires an entry point".to_string(),
        });
    }
    Ok(program)
}

/// CEP:WHAT: Serializes the compiled program as a deterministic ELF64 relocatable.
/// CEP:WHY: The reproducible-build gate compiles twice and compares bytes with cmp.
/// CEP:STATUS: complete
/// CEP:FAILURE: DriverError::Stage on object-emission bounds failures.
/// CEP:ASSUMES: none
/// CEP:COST: O(object bytes).
/// CEP:EVIDENCE: test pipeline_is_deterministic.
pub fn emit_object(program: &CompiledProgram) -> Result<Vec<u8>, DriverError> {
    rv8_object::elf::write_elf64_relocatable(&program.object)
        .map_err(|e| DriverError::Stage { stage: "emit_obj", message: e.to_string() })
}

/// CEP:WHAT: Links a static executable, runs it, and returns its exit code.
/// CEP:WHY: The end-to-end gate: compile fixture, link with a synthesized _start, exec,
///   and observe the program result (13 for the fixture).
/// CEP:STATUS: complete
/// CEP:FAILURE: DriverError::Stage on link failures, DriverError::Io on filesystem or
///   spawn failures, DriverError::ExecSupported off unix, DriverError::ExitMismatch when
///   the session carries --expect and the code differs.
/// CEP:ASSUMES: unix executable semantics (chmod + exec).
/// CEP:COST: O(image bytes) plus process lifetime.
/// CEP:EVIDENCE: test pipeline_runs_fixture_exit_13.
pub fn run(session: &Session, program: &CompiledProgram) -> Result<i32, DriverError> {
    let entry = program
        .entry_symbol
        .clone()
        .ok_or_else(|| DriverError::Stage { stage: "link", message: "missing main".to_string() })?;
    let start = rv8_object::link::synthesize_start(&entry);
    let image = rv8_object::link::link_executable(
        &[program.object.clone(), start],
        "_start",
        session.base_vaddr,
        session.gc_sections,
    )
    .map_err(|e| DriverError::Stage { stage: "link", message: e.to_string() })?;

    #[cfg(not(unix))]
    {
        let _ = &image;
        return Err(DriverError::ExecUnsupported);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // Unique path per invocation: concurrent driver runs (parallel test
        // threads inside one process, concurrent processes) must never share one
        // executable file. Process id separates processes; the atomic counter
        // separates threads. The fixed name made parallel end-to-end tests flake
        // on write/exec races (RV8-137 follow-up).
        //
        // ETXTBSY hardening (2026-11-04 session): CI still observed one
        // "spawn exec: Text file busy (os error 26)" even with unique paths -
        // exec refuses a file ANY process still holds open for writing, which a
        // unique name alone cannot rule out under aggressive test parallelism
        // (write/close visibility windows). The spawn therefore retries on
        // ETXTBSY with a FRESH unique path (bounded retries, never silent), and
        // the temporary executable is removed after the run so a long-lived
        // session can never revisit a live file.
        static RUN_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        const MAX_SPAWN_ATTEMPTS: u32 = 4;
        let mut attempt: u32 = 0;
        let actual = loop {
            attempt += 1;
            let seq = RUN_SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let path = std::env::temp_dir().join(format!(
                "rv8c_run_bin_{}_{}_{}",
                std::process::id(),
                seq,
                attempt
            ));
            std::fs::write(&path, &image)
                .map_err(|e| DriverError::Io(format!("write exec: {e}")))?;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                .map_err(|e| DriverError::Io(format!("chmod exec: {e}")))?;
            let spawned = std::process::Command::new(&path).status();
            // The file's life ends with this invocation; removal happens whether the
            // spawn succeeded or not.
            let remove = std::fs::remove_file(&path);
            match spawned {
                Ok(status) => break status.code().unwrap_or(-1),
                Err(e) if e.raw_os_error() == Some(26) && attempt < MAX_SPAWN_ATTEMPTS => {
                    // ETXTBSY: retire this path, try a fresh one.
                    let _ = remove;
                    continue;
                }
                Err(e) => {
                    let _ = remove;
                    return Err(DriverError::Io(format!("spawn exec: {e}")));
                }
            }
        };
        if let Some(expected) = session.expect_exit {
            if actual != expected {
                return Err(DriverError::ExitMismatch { expected, actual });
            }
        }
        Ok(actual)
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::session::parse_args;

    /// CEP:WHAT: Compiles the repository fixture and runs the linked executable.
    /// CEP:WHY: The project heartbeat: add(6, 7) must exit 13 through the FULL pipeline
    ///   (parse, lower, five IR levels, passes, isel, MCF, encode, link, exec).
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect/assert (test-only).
    /// CEP:ASSUMES: unix exec available.
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    #[cfg(unix)]
    fn pipeline_runs_fixture_exit_13() {
        let src = include_str!("../../../tests/fixtures/add_main.rs");
        let args = vec!["fixture".to_string(), "--emit".to_string(), "run".to_string()];
        let session = parse_args(&args).expect("session");
        let program = compile(&session, src).expect("compile fixture");
        let code = run(&session, &program).expect("run");
        assert_eq!(code, 13, "add(6, 7) must exit 13");
    }

    /// CEP:WHAT: Compiles twice and compares object bytes.
    /// CEP:WHY: STANDARDS 2.2 determinism gate at the driver level.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect/assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn pipeline_is_deterministic() {
        let src = include_str!("../../../tests/fixtures/add_main.rs");
        let session = Session::default();
        let a = compile(&session, src).expect("compile a");
        let b = compile(&session, src).expect("compile b");
        let bytes_a = emit_object(&a).expect("emit a");
        let bytes_b = emit_object(&b).expect("emit b");
        assert_eq!(bytes_a, bytes_b, "same input must give byte-identical objects");
        assert!(a.stages.equals(&b.stages), "stage hashes must match across runs");
    }

    /// CEP:WHAT: Runs a loop program (sum 1..=5) through the whole pipeline.
    /// CEP:WHY: Exercises loop lowering, phi placement, branch resolution, and register
    ///   pressure beyond the straight-line fixture.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect/assert (test-only).
    /// CEP:ASSUMES: unix exec available.
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test. RV8-137 fixed: lane-private MCF job pairs, wrap-sound
    ///   intervals from CFG liveness, threaded return values, and the expression
    ///   flattener operand order (see the RV8-137 regression batch in
    ///   rv8_backend::regalloc::tests and rv8_ir::lowering::tests).
    #[test]
    #[cfg(unix)]
    fn pipeline_runs_loop_program() {
        let src = "fn main() -> i32 { let mut i = 1; let mut acc = 0; \
                   while i < 6 { acc = acc + i; i = i + 1; } acc }";
        let args = vec!["loop.src".to_string(), "--emit".to_string(), "run".to_string()];
        let session = parse_args(&args).expect("session");
        let program = compile(&session, src).expect("compile loop");
        let code = run(&session, &program).expect("run loop");
        assert_eq!(code, 15, "1+2+3+4+5 = 15");
    }

    /// CEP:WHAT: Run mode without a main function is a link-stage error.
    /// CEP:WHY: Fail-closed entry discovery.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn run_mode_requires_main() {
        let src = "fn helper() -> i32 { 7 }";
        let args = vec!["x.src".to_string(), "--emit".to_string(), "run".to_string()];
        let session = parse_args(&args).expect("session");
        let outcome = compile(&session, src);
        let err = match outcome {
            Ok(_) => panic!("must reject missing main"),
            Err(e) => e,
        };
        assert!(err.to_string().contains("main"), "error must name the missing entry");
    }
}
