// CEP:FILE: crates/rv8_driver/src/main.rs
// CEP:WHAT: rv8c command-line entry point: parse the session, read the input, run the
//   pipeline, and produce the requested artifact or run report.
// CEP:WHY: The user-facing rustc-drop-in interface (standard 5.11); exit codes are part
//   of the contract: 0 success, 1 failure, 2 usage.
// CEP:CLASS: HPC-1
// CEP:STATUS: complete
// CEP:FAILURE: prints rv8c: error: <cause> to stderr and exits 1 or 2; never panics on
//   any input.
// CEP:ASSUMES: none
// CEP:COST: the compilation itself.
// CEP:EVIDENCE: CI gates rv8-ci end-to-end compile-and-run and reproducible-build check.

use rv8_driver::error::DriverError;
use rv8_driver::pipeline;
use rv8_driver::session::{self, EmitMode};

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let parsed = session::parse_args(&argv);
    let sess = match parsed {
        Ok(s) => s,
        Err(DriverError::Usage(m)) => {
            eprint!("{}", session::usage_text());
            eprintln!("rv8c: error: {m}");
            std::process::exit(2);
        }
        Err(e) => {
            eprintln!("rv8c: error: {e}");
            std::process::exit(1);
        }
    };
    match drive(&sess) {
        Ok(()) => {}
        Err(e) => {
            eprintln!("rv8c: error: {e}");
            std::process::exit(1);
        }
    }
}

/// CEP:WHAT: Performs the session request: obj emission or compile-and-run.
/// CEP:WHY: Separated from main so the exit-code policy stays in one place and the
///   library remains testable without process exits.
/// CEP:STATUS: complete
/// CEP:FAILURE: propagates DriverError from session stages.
/// CEP:ASSUMES: none
/// CEP:COST: the compilation.
/// CEP:EVIDENCE: CI gates; tests in pipeline.rs.
fn drive(sess: &session::Session) -> Result<(), DriverError> {
    let src = std::fs::read_to_string(&sess.input)
        .map_err(|e| DriverError::Io(format!("read {}: {e}", sess.input)))?;
    let program = pipeline::compile(sess, &src)?;
    match sess.emit {
        EmitMode::Obj => {
            let bytes = pipeline::emit_object(&program)?;
            let path = sess.output.clone().unwrap_or_else(|| "rv8.o".to_string());
            std::fs::write(&path, &bytes)
                .map_err(|e| DriverError::Io(format!("write {path}: {e}")))?;
            println!("wrote {path} ({} bytes)", bytes.len());
            Ok(())
        }
        EmitMode::Run => {
            let code = pipeline::run(sess, &program)?;
            println!("program exited with code {code}");
            Ok(())
        }
    }
}
