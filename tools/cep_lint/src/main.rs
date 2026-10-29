// CEP:FILE: tools/cep_lint/src/main.rs
// CEP:WHAT: Command-line entry point: `cep_lint [--config <path>] <paths...>` lints the
//   given files or directories against the configuration and reports findings plus a
//   summary, exiting clean, findings, bootstrap, or usage per the configured codes.
// CEP:WHY: The CI gate runs this binary over crates/tools/tests; exit-code honesty is the
//   product: bootstrap failures must be distinguishable from real violations.
// CEP:CLASS: CEP-2
// CEP:STATUS: complete
// CEP:OWNER: axiomzero2
// CEP:TICKET: RV8-100
// CEP:FAILURE: configuration or walk failures print a bootstrap diagnostic to stderr and
//   exit with the configured bootstrap code; argument misuse exits with the usage code;
//   findings are printed to stdout sorted by (path, line, rule, message).
// CEP:ASSUMES: invoked from inside the repository (or with an explicit --config path); the
//   repository root is the nearest ancestor containing a .cep directory.
// CEP:COST: one pass over the requested trees; output size is proportional to findings.
// CEP:EVIDENCE: the CI gate run and the integration suite in tests/integration.rs.
// CEP:SECURITY: paths are only read, never written; walking follows no symbolic links.

use cep_lint::{Finding, LintEngine};
use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = run(&args);
    std::process::exit(code);
}

/// CLI dispatch; returns the process exit code.
fn run(args: &[String]) -> i32 {
    let mut config_path: Option<PathBuf> = None;
    let mut paths: Vec<PathBuf> = Vec::new();
    let mut index = 0usize;
    while index < args.len() {
        let arg = args[index].as_str();
        match arg {
            "--help" | "-h" => {
                print_usage();
                return cep_lint::config::ExitCodes::default().clean;
            }
            "--version" | "-v" => {
                println!("cep_lint {} ({})", cep_lint::VERSION, cep_lint::SUMMARY);
                return cep_lint::config::ExitCodes::default().clean;
            }
            "--config" | "-c" => {
                let value = match args.get(index + 1) {
                    Some(value) => value.clone(),
                    None => {
                        eprintln!("cep_lint: usage error: {} requires a path argument", arg);
                        print_usage_short();
                        return cep_lint::config::ExitCodes::default().usage;
                    }
                };
                config_path = Some(PathBuf::from(value));
                index += 2;
            }
            other => {
                if other.starts_with('-') && other.len() > 1 {
                    eprintln!("cep_lint: usage error: unknown option '{}'", other);
                    print_usage_short();
                    return cep_lint::config::ExitCodes::default().usage;
                }
                paths.push(PathBuf::from(other));
                index += 1;
            }
        }
    }
    if paths.is_empty() {
        eprintln!("cep_lint: usage error: at least one path is required");
        print_usage_short();
        return cep_lint::config::ExitCodes::default().usage;
    }
    let engine = match LintEngine::load(config_path.as_deref()) {
        Ok(engine) => engine,
        Err(e) => {
            eprintln!("cep_lint: bootstrap error: {}", e);
            return cep_lint::config::ExitCodes::default().bootstrap;
        }
    };
    let codes = engine.config().exit_codes;
    let outcome = match engine.run(&paths) {
        Ok(outcome) => outcome,
        Err(e) => {
            eprintln!("cep_lint: bootstrap error: {}", e);
            return codes.bootstrap;
        }
    };
    for finding in &outcome.findings {
        println!("{}", finding.render());
    }
    println!("{}", summary(outcome.files, &outcome.findings));
    let failed = outcome
        .findings
        .iter()
        .any(|finding| engine.config().fails(finding.severity));
    if failed {
        codes.findings
    } else {
        codes.clean
    }
}

/// Deterministic summary line over the severity histogram.
fn summary(files: usize, findings: &[Finding]) -> String {
    let mut s0 = 0usize;
    let mut s1 = 0usize;
    let mut s2 = 0usize;
    let mut extra: Vec<(u8, usize)> = Vec::new();
    for finding in findings {
        match finding.severity {
            0 => s0 += 1,
            1 => s1 += 1,
            2 => s2 += 1,
            severity => match extra.iter_mut().find(|(s, _)| *s == severity) {
                Some(slot) => slot.1 += 1,
                None => extra.push((severity, 1)),
            },
        }
    }
    extra.sort();
    let mut line = format!(
        "cep_lint: {} files, {} findings (S0={}, S1={}, S2={}",
        files,
        findings.len(),
        s0,
        s1,
        s2
    );
    for (severity, count) in extra {
        line.push_str(&format!(", S{}={}", severity, count));
    }
    line.push(')');
    line
}

fn print_usage() {
    println!(
        "cep_lint {} - {}\n\nUSAGE:\n    cep_lint [--config <path>] <paths...>\n\nOPTIONS:\n    --config, -c <path>  lint configuration (default: .cep/cep_lint.json\n                         under the repository root found from the working directory)\n    --help, -h           show this help\n    --version, -v        print version\n\nEXIT CODES:\n    0  no failing findings\n    1  findings at a severity listed in fail_on\n    2  bootstrap failure (config missing or unparseable, path walk failure)\n    3  command-line usage error",
        cep_lint::VERSION,
        cep_lint::SUMMARY
    );
}

fn print_usage_short() {
    eprintln!("usage: cep_lint [--config <path>] <paths...>");
}
