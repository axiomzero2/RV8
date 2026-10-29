// CEP:FILE: crates/rv8_driver/src/session.rs
// CEP:WHAT: The immutable compiler session: input path, emission mode, output path, and
//   link toggles, parsed from the rv8c command line.
// CEP:WHY: Standard 5.11 fixes an immutable session as the unit of configuration; every
//   later stage reads it and nothing mutates it, so a run is fully described by its argv.
// CEP:CLASS: HPC-1
// CEP:STATUS: complete
// CEP:FAILURE: parse_args returns DriverError::Usage on any malformed invocation.
// CEP:ASSUMES: argv[0] is the program name and is skipped.
// CEP:COST: O(argc).
// CEP:EVIDENCE: unit tests rv8_driver::session::tests::*.

/// CEP:WHAT: What the driver should produce from one input.
/// CEP:WHY: The CI gauntlet exercises both modes: `--emit obj` for the reproducible-build
///   cmp check and `--emit run` for the end-to-end compile-and-run gate.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: POD.
/// CEP:EVIDENCE: unit tests rv8_driver::session::tests::parse_accepts_both_modes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EmitMode {
    /// Emit a relocatable ELF object for every function in one .o file.
    Obj,
    /// Compile, link a static executable, run it, and report its exit code.
    Run,
}

/// CEP:WHAT: The immutable per-invocation configuration of the compiler.
/// CEP:WHY: One struct threads every decision through the pipeline; determinism follows
///   from the session plus the input bytes alone.
/// CEP:STATUS: complete
/// CEP:FAILURE: construction only via parse_args, which validates every field.
/// CEP:ASSUMES: none
/// CEP:COST: POD.
/// CEP:EVIDENCE: unit tests rv8_driver::session::tests::*.
#[derive(Clone, Debug)]
pub struct Session {
    /// Source file to compile.
    pub input: String,
    /// Requested artifact.
    pub emit: EmitMode,
    /// Output path for EmitMode::Obj (default "rv8.o").
    pub output: Option<String>,
    /// Base virtual address for linked executables.
    pub base_vaddr: u64,
    /// Enable unreachable-section GC during linking.
    pub gc_sections: bool,
    /// If set, --emit run asserts the child exit code and fails on mismatch.
    pub expect_exit: Option<i32>,
}

impl Default for Session {
    /// CEP:WHAT: Default session used by tests: fixture input, obj mode, GC on.
    /// CEP:WHY: Keeps test construction one word short.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: used across rv8_driver tests.
    fn default() -> Session {
        Session {
            input: String::new(),
            emit: EmitMode::Obj,
            output: None,
            base_vaddr: rv8_object::link::DEFAULT_BASE_VADDR,
            gc_sections: true,
            expect_exit: None,
        }
    }
}

/// CEP:WHAT: Parses argv (without the program name) into a Session.
/// CEP:WHY: Fail-closed CLI contract: unknown flags, missing values, and positional
///   overflow are Usage errors, never guesses.
/// CEP:WHY2: Supported forms: `<input> [--emit obj|run] [-o <path>] [--base <hex>]
///   [--no-gc] [--expect <code>]`.
/// CEP:STATUS: complete
/// CEP:FAILURE: DriverError::Usage for unknown flags, missing values, bad numbers, or a
///   second positional argument.
/// CEP:ASSUMES: args excludes argv[0].
/// CEP:COST: O(argc).
/// CEP:EVIDENCE: unit tests rv8_driver::session::tests::*.
pub fn parse_args(args: &[String]) -> Result<Session, crate::error::DriverError> {
    use crate::error::DriverError;
    let mut s = Session::default();
    let mut positional = 0usize;
    let mut i = 0usize;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "--emit" => {
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| DriverError::usage("--emit needs a value (obj|run)"))?;
                s.emit = match v.as_str() {
                    "obj" => EmitMode::Obj,
                    "run" => EmitMode::Run,
                    other => {
                        return Err(DriverError::usage(format!("unknown --emit mode: {other}")))
                    }
                };
                i += 2;
            }
            "-o" | "--output" => {
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| DriverError::usage("-o needs a path"))?;
                s.output = Some(v.to_string());
                i += 2;
            }
            "--base" => {
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| DriverError::usage("--base needs a hex number"))?;
                let hex = v.trim_start_matches("0x");
                s.base_vaddr = u64::from_str_radix(hex, 16)
                    .map_err(|_| DriverError::usage(format!("bad --base: {v}")))?;
                i += 2;
            }
            "--no-gc" => {
                s.gc_sections = false;
                i += 1;
            }
            "--expect" => {
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| DriverError::usage("--expect needs an integer"))?;
                s.expect_exit = Some(
                    v.parse::<i32>()
                        .map_err(|_| DriverError::usage(format!("bad --expect: {v}")))?,
                );
                i += 2;
            }
            other => {
                if other.starts_with('-') {
                    return Err(DriverError::usage(format!("unknown flag: {other}")));
                }
                if positional != 0 {
                    return Err(DriverError::usage("exactly one input file is allowed"));
                }
                s.input = other.to_string();
                positional += 1;
                i += 1;
            }
        }
    }
    if positional == 0 {
        return Err(DriverError::usage("missing input file"));
    }
    if s.emit == EmitMode::Run && s.expect_exit.is_none() {
        // run mode reports the child code; --expect is optional and only tightens it.
    }
    Ok(s)
}

/// CEP:WHAT: The usage banner printed for Usage errors.
/// CEP:WHY: One canonical help text keeps docs and behavior in sync.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: printed by main on DriverError::Usage.
pub fn usage_text() -> String {
    "usage: rv8c <input> [--emit obj|run] [-o <path>] [--base <hex>] [--no-gc] [--expect <code>]\n"
        .to_string()
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// CEP:WHAT: Parses the two CI invocation shapes.
    /// CEP:WHY: The CI gates pin these exact forms; regressions here break the gauntlet.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn parse_accepts_both_modes() {
        let run = parse_args(&["tests/fixtures/add_main.rs".to_string(), "--emit".into(), "run".into()])
            .expect("run form");
        assert_eq!(run.emit, EmitMode::Run);
        assert!(run.gc_sections);
        let obj = parse_args(&[
            "f.src".to_string(),
            "--emit".into(),
            "obj".into(),
            "-o".into(),
            "/tmp/a.o".into(),
        ])
        .expect("obj form");
        assert_eq!(obj.emit, EmitMode::Obj);
        assert_eq!(obj.output.as_deref(), Some("/tmp/a.o"));
    }

    /// CEP:WHAT: Rejects malformed invocations.
    /// CEP:WHY: Fail-closed CLI contract.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn parse_rejects_malformed() {
        assert!(parse_args(&[]).is_err(), "missing input");
        assert!(parse_args(&["a".into(), "--emit".into()]).is_err(), "dangling value");
        assert!(parse_args(&["a".into(), "--bogus".into()]).is_err(), "unknown flag");
        assert!(parse_args(&["a".into(), "b".into()]).is_err(), "two inputs");
        assert!(parse_args(&["a".into(), "--base".into(), "zz".into()]).is_err(), "bad base");
        let with_base = parse_args(&["a".into(), "--base".into(), "0x480000".into()]).expect("hex");
        assert_eq!(with_base.base_vaddr, 0x480000);
    }
}
