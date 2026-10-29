// CEP:FILE: tools/cep_lint/src/lib.rs
// CEP:WHAT: Public API of the CEP lint engine: configuration loading, the LintEngine over a
//   discovered repository root, the free lint_paths entry point, and the Finding type.
// CEP:WHY: The CI gate and the binary share one library so the tool and its tests exercise
//   the same code paths; the API surface stays small enough to audit at a glance.
// CEP:CLASS: CEP-2
// CEP:STATUS: complete
// CEP:OWNER: axiomzero2
// CEP:TICKET: RV8-100
// CEP:FAILURE: fallible operations return Result (bootstrap failures are LintError and
//   ConfigError); the library has no panic path reachable from public functions and all
//   traversals over untrusted structure are iterative with named budgets.
// CEP:ASSUMES: callers pass valid UTF-8 sources and repo-relative slash paths for scoping;
//   the configuration is the first-party .cep/cep_lint.json shape.
// CEP:COST: one linear scan per file plus rule evaluation; no allocation beyond findings.
// CEP:EVIDENCE: cargo test -p cep_lint (unit and integration suites) and the CI gate
//   `cargo run -p cep_lint -- crates tools tests` over the live tree.
// CEP:SECURITY: zero external dependencies; regex and JSON inputs are step/depth budgeted
//   so hostile content cannot drive recursion or unbounded work (38.12, 38.13).

pub mod config;
pub mod engine;
pub mod json;
pub mod regexlite;
pub mod rules;
pub mod scan;
pub mod waivers;

pub use config::{Config, ConfigError, Rule};
pub use engine::{lint_paths, LintEngine, LintError, RunOutcome};
pub use rules::Finding;

/// Crate version for the CLI banner.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// One-line description of what the tool checks, for help text.
pub const SUMMARY: &str = "CEP&CC comment-discipline linter (config-driven)";

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    /// The full compliant-fixture acceptance test required by the tool contract.
    #[test]
    fn api_end_to_end_on_compliant_fixture() {
        let config = Config::from_str(config::tests::SHIPPED_TEXT).expect("shipped config");
        let dir = std::env::temp_dir().join(format!(
            "cep_lint_api_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let file = dir.join("ok.rs");
        std::fs::write(
            &file,
            "// CEP:FILE: ok.rs\n// CEP:WHAT: Compliant.\n// CEP:WHY: Test.\n// CEP:CLASS: CEP-2\n// CEP:STATUS: complete\n// CEP:FAILURE: none\n// CEP:ASSUMES: none\n// CEP:COST: cold.\n// CEP:EVIDENCE: test.\n",
        )
        .expect("write fixture");
        let findings = lint_paths(std::slice::from_ref(&file), &config);
        let failing: Vec<&Finding> = findings.iter().filter(|f| config.fails(f.severity)).collect();
        assert!(failing.is_empty(), "compliant fixture failed: {:?}", failing);
        std::fs::remove_dir_all(&dir).ok();
    }
}
