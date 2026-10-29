// CEP:FILE: tools/cep_lint/tests/integration.rs
// CEP:WHAT: End-to-end suites driving the real binary and the library API over synthetic
//   repository trees: exit-code semantics, waiver honoring and expiry advisories,
//   determinism across repeated runs, budget findings for oversized files, and the
//   optional CEP key-vocabulary rule.
// CEP:WHY: The CI gate trusts this tool's exit codes; the contract (clean 0, findings 1,
//   bootstrap 2, usage 3) must be pinned by executable evidence, not documentation.
// CEP:CLASS: CEP-2
// CEP:STATUS: complete
// CEP:OWNER: axiomzero2
// CEP:TICKET: RV8-100
// CEP:FAILURE: every scenario asserts on concrete exit codes, stdout lines, or finding
//   vectors; failures name the mismatch instead of panicking on unwrapped options.
// CEP:ASSUMES: cargo exposes the binary via CARGO_BIN_EXE_cep_lint; temp directories are
//   writable; the shipped .cep/cep_lint.json shape stays stable.
// CEP:COST: one binary invocation per scenario plus small file writes.
// CEP:EVIDENCE: this suite, run by cargo test -p cep_lint in CI.
// CEP:SECURITY: writes are confined to fresh temporary directories.
#![cfg(test)]

use cep_lint::{lint_paths, Config, LintEngine};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::FromStr;

const SHIPPED_CONFIG: &str = include_str!("../../../.cep/cep_lint.json");

/// Fresh unique scratch directory for one scenario.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "cep_lint_itg_{}_{}_{}",
        name,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn write(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create parent dir");
    }
    std::fs::write(path, content).expect("write fixture file");
}

fn write_standard(root: &Path) {
    // The shipped config carries the DOC-SYNC rule; the synthetic repo needs a document
    // with one heading per configured rule and the version marker.
    let mut doc = String::from("# CEP&CC 0.1\n\nVersion: CEP&CC 0.1\n");
    for id in CONFIGURED_RULE_IDS {
        doc.push_str("\n### Rule ");
        doc.push_str(id);
        doc.push('\n');
    }
    write(&root.join("standard.md"), &doc);
}

fn write_config(root: &Path) -> PathBuf {
    let path = root.join(".cep").join("cep_lint.json");
    write(&path, SHIPPED_CONFIG);
    write_standard(root);
    path
}

const CONFIGURED_RULE_IDS: [&str; 17] = [
    "CEP-LINT-FILE-HEADER",
    "CEP-LINT-FUNC-BLOCK",
    "CEP-LINT-STATUS-VALUE",
    "CEP-LINT-TODO-OWNER",
    "CEP-LINT-STUB-DECL",
    "CEP-LINT-PLACEHOLDER-COMMENT",
    "CEP-LINT-PLACEHOLDER-TEXT",
    "CEP-LINT-MAGIC-NUMBER",
    "CEP-LINT-HOT-BANNED",
    "CEP-LINT-STUB-BODY",
    "CEP-LINT-EMPTY-CATCH",
    "CEP-LINT-EMPTY-BRANCH",
    "CEP-LINT-CONSTANT-RETURN",
    "CEP-LINT-NAMING-TYPE",
    "CEP-LINT-NAMING-FUNC",
    "CEP-LINT-COMMENTED-CODE",
    "CEP-LINT-DOC-SYNC",
];

const GOOD_RS: &str = "\
// CEP:FILE: crates/demo/good.rs
// CEP:WHAT: A compliant module.
// CEP:WHY: Shows the accepted shape.
// CEP:CLASS: CEP-2
// CEP:STATUS: complete
// CEP:FAILURE: none
// CEP:ASSUMES: none
// CEP:COST: cold.
// CEP:EVIDENCE: this suite.
";

const BAD_RS: &str = "\
// CEP:FILE: crates/demo/bad.rs
// CEP:WHAT: A module missing most header fields.
// CEP:CLASS: CEP-2
";

fn run_cli(cwd: &Path, args: &[&str]) -> (i32, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_cep_lint"))
        .current_dir(cwd)
        .args(args)
        .output()
        .expect("spawn cep_lint binary");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
    )
}

#[test]
fn clean_tree_exits_zero_with_summary() {
    let root = scratch("clean");
    write_config(&root);
    write(&root.join("crates/demo/good.rs"), GOOD_RS);
    let (code, stdout, stderr) = run_cli(&root, &["crates"]);
    assert_eq!(code, 0, "stdout: {}\nstderr: {}", stdout, stderr);
    assert!(
        stdout.contains("cep_lint: 1 files, 0 findings (S0=0, S1=0, S2=0)"),
        "summary missing: {}",
        stdout
    );
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn violations_exit_one_and_render_sorted() {
    let root = scratch("violations");
    write_config(&root);
    write(&root.join("crates/demo/bad.rs"), BAD_RS);
    write(&root.join("crates/demo/good.rs"), GOOD_RS);
    let (code, stdout, _stderr) = run_cli(&root, &["crates"]);
    assert_eq!(code, 1);
    assert!(stdout.contains("S1 crates/demo/bad.rs:1: CEP-LINT-FILE-HEADER"), "{}", stdout);
    assert!(stdout.contains("cep_lint: 2 files, "), "{}", stdout);
    // Deterministic order: bad.rs findings precede good.rs (both sorted by path).
    let bad_at = stdout.find("bad.rs").expect("bad.rs in output");
    let good_at = stdout.find("good.rs").unwrap_or(usize::MAX);
    assert!(bad_at < good_at);
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn bootstrap_exit_two_without_repository_root() {
    let root = scratch("bootstrap");
    let (code, _stdout, stderr) = run_cli(&root, &["crates"]);
    assert_eq!(code, 2);
    assert!(stderr.contains("bootstrap error"), "{}", stderr);
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn usage_exit_three_without_paths() {
    let root = scratch("usage");
    let (code, _stdout, stderr) = run_cli(&root, &[]);
    assert_eq!(code, 3);
    assert!(stderr.contains("usage error"), "{}", stderr);
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn config_flag_overrides_default_location() {
    let root = scratch("configflag");
    std::fs::create_dir_all(root.join(".cep")).expect("root cep dir");
    write_standard(&root);
    let cfg_dir = root.join("cfg");
    std::fs::create_dir_all(&cfg_dir).expect("cfg dir");
    let cfg_path = cfg_dir.join("other.json");
    write(&cfg_path, SHIPPED_CONFIG);
    write(&root.join("crates/demo/good.rs"), GOOD_RS);
    let (code, stdout, stderr) =
        run_cli(&root, &["--config", cfg_path.to_str().expect("utf8"), "crates"]);
    assert_eq!(code, 0, "stdout: {}\nstderr: {}", stdout, stderr);
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn waiver_suppresses_rule_until_expiry() {
    let root = scratch("waiver");
    write_config(&root);
    write(&root.join("crates/demo/bad.rs"), BAD_RS);
    let shipped = Config::from_str(SHIPPED_CONFIG).expect("config");
    let crates_dir = root.join("crates");

    // Without a waiver the header rule fires.
    let engine = LintEngine::with_parts(shipped.clone(), root.clone());
    let findings = engine
        .run(std::slice::from_ref(&crates_dir))
        .expect("run")
        .findings;
    assert!(
        findings.iter().any(|f| f.rule == "CEP-LINT-FILE-HEADER" && f.severity == 1),
        "expected header finding: {:?}",
        findings
    );

    // An active waiver suppresses it for the scoped path.
    write(
        &root.join(".cep/waivers/CEP-1-CEP-LINT-FILE-HEADER.md"),
        "# CEP-1 header waiver\nEXPIRES: 2999-12-31\nSCOPE: crates/**\nJUSTIFICATION: reviewed in this suite\n",
    );
    let engine = LintEngine::with_parts(shipped.clone(), root.clone());
    let findings = engine
        .run(std::slice::from_ref(&crates_dir))
        .expect("run")
        .findings;
    assert!(
        !findings.iter().any(|f| f.rule == "CEP-LINT-FILE-HEADER"),
        "waiver must suppress: {:?}",
        findings
    );

    // An expired waiver suppresses nothing and is reported at severity 2.
    write(
        &root.join(".cep/waivers/CEP-1-CEP-LINT-FILE-HEADER.md"),
        "# CEP-1 header waiver\nEXPIRES: 2000-01-01\nSCOPE: crates/**\nJUSTIFICATION: stale\n",
    );
    let engine = LintEngine::with_parts(shipped, root.clone());
    let findings = engine.run(&[crates_dir]).expect("run").findings;
    assert!(
        findings.iter().any(|f| f.rule == "CEP-LINT-FILE-HEADER" && f.severity == 1),
        "expired waiver must not suppress: {:?}",
        findings
    );
    assert!(
        findings.iter().any(|f| f.rule == "CEP-LINT-BUDGET" && f.message.contains("expired")),
        "expired waiver advisory missing: {:?}",
        findings
    );
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn same_input_yields_identical_findings_twice() {
    let root = scratch("determinism");
    write_config(&root);
    write(&root.join("crates/demo/bad.rs"), BAD_RS);
    write(&root.join("crates/demo/good.rs"), GOOD_RS);
    let config = Config::from_str(SHIPPED_CONFIG).expect("config");
    let first = lint_paths(&[root.join("crates")], &config);
    let second = lint_paths(&[root.join("crates")], &config);
    assert_eq!(first, second, "repeated runs must be identical");
    assert!(!first.is_empty());
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn oversized_file_yields_budget_finding_not_blowup() {
    let root = scratch("budget");
    write_config(&root);
    let big = "a".repeat(cep_lint::scan::MAX_FILE_BYTES as usize + 1);
    write(&root.join("crates/demo/big.rs"), &big);
    let config = Config::from_str(SHIPPED_CONFIG).expect("config");
    let findings = lint_paths(&[root.join("crates")], &config);
    let rules_seen: Vec<String> = findings.iter().map(|f| f.rule.clone()).collect();
    assert!(
        findings.iter().any(|f| f.rule == "CEP-LINT-BUDGET" && f.severity == 1),
        "budget finding missing: {:?}",
        rules_seen
    );
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn key_vocabulary_rule_flags_unknown_keys() {
    let root = scratch("vocab");
    let config_text = "\
{
  \"fail_on\": [0, 1],
  \"exit_codes\": {\"clean\": 0, \"findings\": 1, \"bootstrap\": 2, \"usage\": 3},
  \"file_globs\": [\"crates/**/*.rs\"],
  \"rules\": {
    \"CEP-LINT-KEY-VOCAB\": {
      \"severity\": 2,
      \"check\": \"key_vocabulary\",
      \"allowed_fields\": [\"CEP:FILE\", \"CEP:WHAT\", \"CEP:STATUS\"]
    }
  }
}
";
    write(&root.join(".cep/cep_lint.json"), config_text);
    let vocab_source = GOOD_RS.replace("CEP:WHY", "CEP:MYSTERY");
    write(&root.join("crates/demo/vocab.rs"), &vocab_source);
    let config = Config::from_str(config_text).expect("config");
    let findings = lint_paths(&[root.join("crates")], &config);
    assert!(
        findings
            .iter()
            .any(|f| f.rule == "CEP-LINT-KEY-VOCAB" && f.message.contains("CEP:MYSTERY")),
        "unknown key finding missing: {:?}",
        findings
    );
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn continuation_lines_and_exemptions_end_to_end() {
    let root = scratch("exemptions");
    write_config(&root);
    let file = "\
// CEP:FILE: crates/demo/exempt.rs
// CEP:WHAT: Re-exports and test mods are exempt from item blocks,
//   and header values continue across lines like this one.
// CEP:WHY: Proves the exemption paths end to end.
// CEP:CLASS: CEP-2
// CEP:STATUS: complete
// CEP:FAILURE: none
// CEP:ASSUMES: none
// CEP:COST: cold.
// CEP:EVIDENCE: this suite.
pub use crate::other::{Alpha, Beta};

#[cfg(test)]
pub mod tests {
    pub fn hidden() -> u32 {
        let value = 41;
        value + 1
    }
}
";
    write(&root.join("crates/demo/exempt.rs"), file);
    let (code, stdout, stderr) = run_cli(&root, &["crates"]);
    assert_eq!(code, 0, "stdout: {}\nstderr: {}", stdout, stderr);
    std::fs::remove_dir_all(&root).ok();
}
