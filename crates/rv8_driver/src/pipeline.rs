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
use crate::query::StageHashes;
use crate::session::{EmitMode, Session};
use rv8_object::model::{ObjectFile, RelocKind as ObjRelocKind};

/// CEP:WHAT: Renders a rv8_ir lowering error for a driver diagnostic.
/// CEP:WHY: LowerError implements Debug but not Display; the driver formats without
///   requiring changes to the frozen IR crate. Upgrade ticket: RV8-135.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: LowerError derives Debug (it does).
/// CEP:COST: allocation on error paths.
/// CEP:EVIDENCE: used by compile in this module.
fn ir_msg(e: rv8_ir::LowerError) -> String {
    format!("{e:?}")
}

/// CEP:WHAT: The packed compilation output: one relocatable object holding every function
///   plus the stage-hash log gathered on the way.
/// CEP:WHY: Run mode needs the object for linking; obj mode serializes it directly; the
///   stage log is the determinism evidence trail.
/// CEP:STATUS: complete
/// CEP:FAILURE: construction only through compile.
/// CEP:ASSUMES: none
/// CEP:COST: owns the object bytes.
/// CEP:EVIDENCE: tests in this module.
pub struct CompiledProgram {
    /// One object containing one `.text.fn<i>` section per function.
    pub object: ObjectFile,
    /// Real name of the entry function when one is named main.
    pub entry_symbol: Option<String>,
    /// Per-level canonical hashes recorded during compilation.
    pub stages: crate::query::StageLog,
}

/// CEP:WHAT: Compiles source text into a packed object through the whole pipeline.
/// CEP:WHY: The single entry point main.rs calls; stage order is the normative pipeline.
/// CEP:STATUS: complete
/// CEP:FAILURE: DriverError::Stage naming the failing stage on any rejection.
/// CEP:ASSUMES: src is UTF-8 Rust subset text.
/// CEP:COST: linear in source size across all stages.
/// CEP:EVIDENCE: test pipeline_runs_fixture_exit_13.
pub fn compile(session: &Session, src: &str) -> Result<CompiledProgram, DriverError> {
    let hirs = rv8_frontend::compile_source(src)
        .map_err(|e| DriverError::Stage { stage: "frontend", message: e.to_string() })?;
    let mut program = ObjectFile::new("rv8");
    let mut stages = crate::query::StageLog::new();
    let mut entry_symbol: Option<String> = None;

    for (ordinal, hir) in hirs.iter().enumerate() {
        let fn_name = hir.name.clone();
        if fn_name == "main" {
            entry_symbol = Some(fn_name.clone());
        }
        // Level 0 -> Level 1: flatten the region tree into the Sea of Nodes.
        let mut sea = rv8_ir::lowering::hir_to_son(hir)
            .map_err(|e| DriverError::Stage { stage: "hir_to_son", message: ir_msg(e) })?;
        let son_hash = sea.canonical_hash();
        // Borrow checking (graph CSP) before optimization mutates anything.
        let borrow = rv8_middle::borrowck::check(&sea)
            .map_err(|e| DriverError::Stage { stage: "borrowck", message: e.to_string() })?;
        if !borrow.conflicts.is_empty() {
            return Err(DriverError::Stage {
                stage: "borrowck",
                message: format!("{:?}", borrow.conflicts),
            });
        }
        // Middle end: constant folding, GVN, DCE; verify after every pass inside.
        let _summary_son = rv8_middle::pipeline::optimize_son(&mut sea)
            .map_err(|e| DriverError::Stage { stage: "optimize_son", message: e.to_string() })?;
        let son_opt_hash = sea.canonical_hash();
        // Level 1 -> Level 2: linearize into the Heterogeneous IR.
        let mut het = rv8_ir::lower_het::son_to_hetir(&sea)
            .map_err(|e| DriverError::Stage { stage: "son_to_hetir", message: ir_msg(e) })?;
        let het_hash = het.canonical_hash();
        // HetIR dead-node elimination per spec 3.3.4.
        let _summary_het = rv8_middle::pipeline::optimize_hetir(&mut het)
            .map_err(|e| DriverError::Stage { stage: "optimize_hetir", message: e.to_string() })?;
        let het_opt_hash = het.canonical_hash();
        // Level 2 -> Level 3: target-independent virtual-register LIR.
        let lir = rv8_ir::lower_het::hetir_to_lir(&het, &fn_name)
            .map_err(|e| DriverError::Stage { stage: "hetir_to_lir", message: ir_msg(e) })?;
        // Level 3 -> Level 4 -> bytes: isel, MCF allocation, scheduling, encoding.
        let symbol = rv8_backend::encode::call_symbol_name(ordinal as i64);
        let out = rv8_backend::compile_lir(&lir, &symbol)
            .map_err(|e| DriverError::Stage { stage: "backend", message: e.to_string() })?;
        let mir_hash = rv8_core::hash::fxhash64(&out.code);

        // Pack: one section per function; callers relocate against fn{ordinal}, and the
        // real function name is an alias symbol at the same address so the linker and
        // synthesize_start can use plain names.
        let sec = program
            .add_text_section(&format!(".text.{symbol}"), 16, out.code.clone())
            .map_err(|e| DriverError::Stage { stage: "object", message: e.to_string() })?;
        for r in &out.relocs {
            let kind = match r.kind {
                rv8_backend::encode::RelocKind::Abs64 => ObjRelocKind::Abs64,
                rv8_backend::encode::RelocKind::PcRel32 => ObjRelocKind::PcRel32,
            };
            program
                .add_reloc(sec, r.offset, kind, &r.symbol, r.addend)
                .map_err(|e| DriverError::Stage { stage: "object", message: e.to_string() })?;
        }
        program
            .add_symbol(&symbol, true, true, Some(sec), 0, out.code.len() as u64)
            .map_err(|e| DriverError::Stage { stage: "object", message: e.to_string() })?;
        program
            .add_symbol(&fn_name, true, true, Some(sec), 0, out.code.len() as u64)
            .map_err(|e| DriverError::Stage { stage: "object", message: e.to_string() })?;

        stages.record(StageHashes {
            function: fn_name,
            son: son_hash,
            son_opt: son_opt_hash,
            het: het_hash,
            het_opt: het_opt_hash,
            mir: mir_hash,
        });
    }

    if session.emit == EmitMode::Run && entry_symbol.is_none() {
        return Err(DriverError::Stage {
            stage: "link",
            message: "no function named main: run mode requires an entry point".to_string(),
        });
    }
    Ok(CompiledProgram { object: program, entry_symbol, stages })
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
        static RUN_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let seq = RUN_SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!(
            "rv8c_run_bin_{}_{}",
            std::process::id(),
            seq
        ));
        std::fs::write(&path, &image)
            .map_err(|e| DriverError::Io(format!("write exec: {e}")))?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| DriverError::Io(format!("chmod exec: {e}")))?;
        let status = std::process::Command::new(&path)
            .status()
            .map_err(|e| DriverError::Io(format!("spawn exec: {e}")))?;
        let actual = status.code().unwrap_or(-1);
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
