// CEP:FILE: tools/cep_lint/src/engine.rs
// CEP:WHAT: The lint engine: repository-root discovery, budgeted iterative directory
//   walking, glob scoping, per-file scanning and rule evaluation, doc-sync checking,
//   waiver application, and deterministic finding ordering.
// CEP:WHY: CI gates need one entry point with honest exit semantics: bootstrap failures
//   (exit 2) for environment/config problems, findings (exit 1) only for violations at
//   configured severities, and clean (exit 0) otherwise.
// CEP:CLASS: CEP-2
// CEP:STATUS: complete
// CEP:OWNER: axiomzero2
// CEP:TICKET: RV8-100
// CEP:FAILURE: unreadable paths and unreadable files return Err (bootstrap); oversized
//   files and file-count overruns produce named budget findings instead of unbounded work;
//   findings are sorted by (path, line, rule, message) so output is byte-stable.
// CEP:ASSUMES: the repository root is the nearest ancestor of the working directory (or of
//   the config file) containing a `.cep` directory; paths outside the root are reported
//   under their given spelling.
// CEP:COST: O(tree bytes) with one scan per file; walking is iterative with an explicit
//   stack; nothing allocates proportional to directory depth.
// CEP:EVIDENCE: integration tests drive discovery, budget findings, determinism, waivers,
//   and the real-tree clean gate; the CI run is the standing evidence.
// CEP:SECURITY: symbolic links are never followed; walk breadth and file sizes are bounded
//   by named constants (38.12, 38.13).

use crate::config::{any_glob_match, Config, Rule};
use crate::json;
use crate::regexlite::Regex;
use crate::rules::{self, Finding};
use crate::scan::{self, MAX_FILE_BYTES};
use crate::waivers;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;

/// Upper bound on considered files per run; exceeding it yields a budget finding.
pub const MAX_FILES: usize = 50_000;

/// Rule id used for engine-level budget findings.
pub const BUDGET_RULE: &str = "CEP-LINT-BUDGET";

/// CEP:WHAT: Engine failure of the bootstrap class: the environment could not support a
///   real run (root not found, config unreadable, path walk error).
/// CEP:WHY: Exit-code honesty: bootstrap problems must surface as a distinct diagnostic and
///   exit code, never as findings or panics.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; construction is infallible and Display renders the message.
/// CEP:ASSUMES: messages are human-readable first-party diagnostics.
/// CEP:COST: one String per error, failure path only.
/// CEP:EVIDENCE: integration::bootstrap_exit_two_without_repository_root asserts the
///   message and exit code through the binary.
#[derive(Debug, Clone, PartialEq)]
pub struct LintError {
    pub message: String,
}

impl LintError {
    fn new(message: impl Into<String>) -> LintError {
        LintError {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for LintError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for LintError {}

/// CEP:WHAT: A loaded lint engine bound to one configuration and repository root, with the
///   waiver set loaded from .cep/waivers.
/// CEP:WHY: CI needs one object whose run() has deterministic, honest semantics over the
///   whole tree; binding root and policy once makes every run reproducible.
/// CEP:STATUS: complete
/// CEP:FAILURE: run returns Err for stat/read failures (bootstrap class); budget overruns
///   become findings instead of unbounded work.
/// CEP:ASSUMES: the repository root is the nearest ancestor containing a .cep directory.
/// CEP:COST: O(tree bytes) per run; walking is iterative with sorted directory reads.
/// CEP:EVIDENCE: integration suites drive clean, violations, waiver, budget, and
///   determinism scenarios through run(); the CI gate is standing evidence.
pub struct LintEngine {
    config: Config,
    root: PathBuf,
    waivers: Vec<waivers::Waiver>,
}

/// CEP:WHAT: Result of one engine run: the deterministically sorted findings plus the
///   number of in-scope files examined.
/// CEP:WHY: The summary line and the exit decision both need exactly these two facts.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; a plain data carrier.
/// CEP:ASSUMES: findings are already sorted and waiver-filtered when produced by run().
/// CEP:COST: two words plus the findings vector.
/// CEP:EVIDENCE: asserted in integration::clean_tree_exits_zero_with_summary and the
///   determinism suite.
#[derive(Debug, Clone, PartialEq)]
pub struct RunOutcome {
    pub findings: Vec<Finding>,
    pub files: usize,
}

impl LintEngine {
    /// CEP:WHAT: Loads the configuration (explicit path or the default .cep/cep_lint.json)
    ///   and the waiver directory, discovering the repository root by walking upward.
    /// CEP:WHY: The binary must work from any working directory inside the repository and
    ///   fail closed with a bootstrap error when no root exists.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: Err when no ancestor contains .cep or the configuration cannot be
    ///   loaded; the binary maps this to the configured bootstrap code.
    /// CEP:ASSUMES: an explicit --config path's parent participates in root discovery
    ///   before the working directory.
    /// CEP:COST: O(depth) stat calls for discovery plus one config parse.
    /// CEP:EVIDENCE: integration::bootstrap_exit_two_without_repository_root and
    ///   integration::config_flag_overrides_default_location.
    pub fn load(config_path: Option<&Path>) -> Result<LintEngine, LintError> {
        let root = discover_root(config_path)?;
        let default = root.join(".cep").join("cep_lint.json");
        let path = match config_path {
            Some(explicit) => explicit.to_path_buf(),
            None => default,
        };
        let config = Config::from_file(&path)
            .map_err(|e| LintError::new(format!("cannot load configuration: {}", e)))?;
        let waiver_dir = root.join(".cep").join("waivers");
        let loaded = waivers::load(&waiver_dir);
        Ok(LintEngine {
            config,
            root,
            waivers: loaded,
        })
    }

    /// CEP:WHAT: Engine bound to an already-parsed configuration and root, loading waivers
    ///   from the root's .cep/waivers directory.
    /// CEP:WHY: Tests and embedded callers need engine semantics without on-disk config
    ///   discovery.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none; a missing waiver directory simply means no waivers.
    /// CEP:ASSUMES: root points at a directory treated as the repository root.
    /// CEP:COST: one waiver-directory read.
    /// CEP:EVIDENCE: integration::waiver_suppresses_rule_until_expiry builds engines with
    ///   this constructor for all three waiver scenarios.
    pub fn with_parts(config: Config, root: PathBuf) -> LintEngine {
        let waiver_dir = root.join(".cep").join("waivers");
        LintEngine {
            config,
            root,
            waivers: waivers::load(&waiver_dir),
        }
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// CEP:WHAT: Runs every configured rule over the given paths, applies waivers, and
    ///   returns findings sorted by (path, line, rule, message) with the file count.
    /// CEP:WHY: This is the contract the CI gate executes; its output must be byte-stable
    ///   and its errors must separate bootstrap failures from findings.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: Err (bootstrap) for stat/read failures; oversized files and file-count
    ///   overruns produce named CEP-LINT-BUDGET findings instead of unbounded work.
    /// CEP:ASSUMES: paths may be files or directories; symbolic links are never followed.
    /// CEP:COST: O(tree bytes) with one scan per file; sort is O(F log F) on findings.
    /// CEP:EVIDENCE: the integration suite pins sorting, waivers, budgets, and the exit
    ///   codes; the real-tree gate runs this on every CI pass.
    pub fn run(&self, paths: &[PathBuf]) -> Result<RunOutcome, LintError> {
        let mut files = Vec::new();
        let mut budget: Vec<Finding> = Vec::new();
        for path in paths {
            self.collect(path, &mut files, &mut budget)?;
        }
        files.sort();
        let mut findings = budget;
        for rel in &files {
            let full = self.root.join(rel);
            let meta = match std::fs::metadata(&full) {
                Ok(meta) => meta,
                Err(e) => {
                    return Err(LintError::new(format!(
                        "cannot stat {}: {}",
                        full.display(),
                        e
                    )));
                }
            };
            if meta.len() > MAX_FILE_BYTES {
                findings.push(Finding::new(
                    1,
                    rel,
                    1,
                    BUDGET_RULE,
                    format!(
                        "file is {} bytes, above the {} scan budget; not linted",
                        meta.len(),
                        MAX_FILE_BYTES
                    ),
                ));
                continue;
            }
            let source = match std::fs::read_to_string(&full) {
                Ok(source) => source,
                Err(e) => {
                    return Err(LintError::new(format!(
                        "cannot read {}: {}",
                        full.display(),
                        e
                    )));
                }
            };
            let scanned = scan::scan(&source);
            findings.extend(rules::run_file_rules(&self.config, rel, &scanned));
            if let Some(line) = scanned.long_line {
                findings.push(Finding::new(
                    2,
                    rel,
                    line,
                    BUDGET_RULE,
                    format!(
                        "line exceeds the {} byte line budget; analysis of this line is truncated",
                        scan::MAX_LINE_BYTES
                    ),
                ));
            }
        }
        findings.extend(self.doc_sync_findings()?);
        let today = waivers::today();
        let mut kept = Vec::with_capacity(findings.len());
        for finding in findings {
            let suppressed = self
                .waivers
                .iter()
                .any(|w| {
                    waivers::is_active(w, today) && waivers::suppresses(w, &finding.rule, &finding.path)
                });
            if !suppressed {
                kept.push(finding);
            }
        }
        for w in &self.waivers {
            if w.rule.is_empty() || !waivers::is_active(w, today) {
                let reason = if w.justification.starts_with("malformed") {
                    w.justification.clone()
                } else {
                    format!("waiver {} is expired or incomplete; it suppresses nothing", w.file)
                };
                kept.push(Finding::new(2, w.file.as_str(), 1, BUDGET_RULE, reason));
            }
        }
        kept.sort();
        Ok(RunOutcome {
            files: files.len(),
            findings: kept,
        })
    }

    /// DOC-SYNC: configured rules must have headings in the standard document and the
    /// document must carry the configured version marker.
    fn doc_sync_findings(&self) -> Result<Vec<Finding>, LintError> {
        let mut findings = Vec::new();
        for rule in self.config.rules_with_check("doc_rule_coverage") {
            let doc_path = match rule.doc_path.as_deref() {
                Some(path) => path,
                None => continue,
            };
            let full = self.root.join(doc_path);
            let text = std::fs::read_to_string(&full).map_err(|e| {
                LintError::new(format!(
                    "doc-rule rule {} cannot read {}: {}",
                    rule.id,
                    full.display(),
                    e
                ))
            })?;
            findings.extend(doc_sync(rule, &self.config, doc_path, &text));
        }
        Ok(findings)
    }

    /// Iteratively collects in-scope files under `path` as root-relative slash paths.
    fn collect(
        &self,
        path: &Path,
        files: &mut Vec<String>,
        budget: &mut Vec<Finding>,
    ) -> Result<(), LintError> {
        let meta = std::fs::symlink_metadata(path)
            .map_err(|e| LintError::new(format!("cannot access {}: {}", path.display(), e)))?;
        if meta.is_symlink() {
            return Ok(());
        }
        // Absolute form so root-relative reporting works for relative arguments too.
        let path = std::fs::canonicalize(path)
            .map_err(|e| LintError::new(format!("cannot resolve {}: {}", path.display(), e)))?;
        if meta.is_file() {
            if let Some(rel) = self.relativize(&path) {
                if any_glob_match(&self.config.file_globs, &rel) {
                    if files.len() >= MAX_FILES {
                        budget.push(Finding::new(
                            1,
                            rel.as_str(),
                            1,
                            BUDGET_RULE,
                            format!("file count exceeds MAX_FILES ({})", MAX_FILES),
                        ));
                        return Ok(());
                    }
                    files.push(rel);
                }
            }
            return Ok(());
        }
        if meta.is_dir() {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            if name == "target" || name == ".git" {
                return Ok(());
            }
            let mut stack: Vec<PathBuf> = vec![path.to_path_buf()];
            while let Some(dir) = stack.pop() {
                let entries = std::fs::read_dir(&dir)
                    .map_err(|e| LintError::new(format!("cannot read dir {}: {}", dir.display(), e)))?;
                let mut names: Vec<PathBuf> = entries
                    .filter_map(|entry| entry.ok())
                    .map(|entry| entry.path())
                    .collect();
                names.sort();
                for entry in names {
                    let meta = match std::fs::symlink_metadata(&entry) {
                        Ok(meta) => meta,
                        Err(_) => continue,
                    };
                    if meta.is_symlink() {
                        continue;
                    }
                    if meta.is_dir() {
                        let base = entry
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or_default();
                        if base == "target" || base == ".git" {
                            continue;
                        }
                        stack.push(entry);
                    } else if meta.is_file() {
                        if let Some(rel) = self.relativize(&entry) {
                            if any_glob_match(&self.config.file_globs, &rel) {
                                if files.len() >= MAX_FILES {
                                    budget.push(Finding::new(
                                        1,
                                        rel.as_str(),
                                        1,
                                        BUDGET_RULE,
                                        format!(
                                            "file count exceeds MAX_FILES ({})",
                                            MAX_FILES
                                        ),
                                    ));
                                    return Ok(());
                                }
                                files.push(rel);
                            }
                        }
                    }
                }
            }
            return Ok(());
        }
        Ok(())
    }

    /// Repo-relative slash path for files under the root; None otherwise (kept as-is).
    fn relativize(&self, path: &Path) -> Option<String> {
        match path.strip_prefix(&self.root) {
            Ok(rel) => Some(rel.to_string_lossy().replace('\\', "/")),
            Err(_) => None,
        }
    }
}

/// Finds the nearest ancestor (starting at the config path's parent, then the working
/// directory) that contains a `.cep` directory.
fn discover_root(config_path: Option<&Path>) -> Result<PathBuf, LintError> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(explicit) = config_path {
        if let Some(parent) = explicit.parent() {
            if let Ok(abs) = std::fs::canonicalize(parent) {
                candidates.push(abs);
            } else {
                candidates.push(parent.to_path_buf());
            }
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd);
    }
    for start in candidates {
        let mut current: Option<&Path> = Some(start.as_path());
        while let Some(dir) = current {
            if dir.join(".cep").is_dir() {
                return Ok(dir.to_path_buf());
            }
            current = dir.parent();
        }
    }
    Err(LintError::new(
        "cannot locate the repository root (no ancestor contains a .cep directory)",
    ))
}

/// CEP:WHAT: DOC-SYNC evaluation over the standard document text: every configured rule id
///   must be documented exactly once, every documented rule id must be configured, and the
///   document's `Version:` line must equal the configuration's standard.version.
/// CEP:WHY: Law 8 (no stale documentation) is enforced mechanically in BOTH coverage
///   directions plus the version contract; the standard and the config must agree or the
///   run fails.  The version check is anchored to the `Version:` metadata line because the
///   previous any-line pattern search was satisfied by a stale cross-reference mentioning
///   an old version elsewhere in the document (the CEP&CC 0.2/0.1 drift defect).
/// CEP:STATUS: complete
/// CEP:FAILURE: findings name duplicate or missing rule headings, documented rules without
///   a configuration entry, and a missing or mismatched `Version:` line; an optional
///   version_pattern (when configured) additionally requires a matching line anywhere;
///   pattern-compile failures degrade to no findings (already rejected at config
///   validation).
/// CEP:ASSUMES: the document uses `### Rule <ID>` headings and carries the version in a
///   `Version:` metadata line (markdown emphasis is trimmed).
/// CEP:COST: O(document lines) per configured doc-sync rule.
/// CEP:EVIDENCE: engine::tests pin the shipped document/config agreement, both coverage
///   directions, heading uniqueness, and the version-equality and missing-version failure
///   paths.
pub fn doc_sync(rule: &Rule, config: &Config, doc_path: &str, text: &str) -> Vec<Finding> {
    let mut findings = Vec::new();
    let heading_pattern = match rule.heading_pattern.as_deref() {
        Some(pattern) => pattern,
        None => return findings,
    };
    let heading_re = match Regex::new(heading_pattern, rule.icase) {
        Ok(re) => re,
        Err(_) => return findings,
    };
    let mut ids: Vec<String> = Vec::new();
    for line in text.lines() {
        if let Some(m) = heading_re.find(line) {
            if let Some(Some((start, end))) = m.groups.get(1) {
                let chars: Vec<char> = line.chars().collect();
                if *end <= chars.len() && *start <= *end {
                    ids.push(chars[*start..*end].iter().collect());
                }
            }
        }
    }
    let mut sorted_ids = ids.clone();
    sorted_ids.sort();
    for window in sorted_ids.windows(2) {
        if window[0] == window[1] {
            findings.push(Finding::new(
                rule.severity,
                doc_path,
                1,
                &rule.id,
                format!("duplicate '### Rule {}' heading in {}", window[0], doc_path),
            ));
        }
    }
    let mut missing: Vec<&String> = Vec::new();
    for configured_rule in &config.rules {
        if !ids.contains(&configured_rule.id) {
            missing.push(&configured_rule.id);
        }
    }
    if !missing.is_empty() {
        let names: Vec<&str> = missing.iter().map(|s| s.as_str()).collect();
        findings.push(Finding::new(
            rule.severity,
            doc_path,
            1,
            &rule.id,
            format!(
                "configured rules without a '### Rule' heading in {}: {}",
                doc_path,
                names.join(", ")
            ),
        ));
    }
    // The reverse coverage direction: every documented rule must exist in the
    // configuration.  A rule documented here but unconfigured is exactly the Law 8
    // drift this rule exists to catch.
    let mut unconfigured: Vec<&str> = Vec::new();
    for id in &ids {
        if !config.rules.iter().any(|configured| &configured.id == id) {
            unconfigured.push(id);
        }
    }
    unconfigured.sort();
    if !unconfigured.is_empty() {
        findings.push(Finding::new(
            rule.severity,
            doc_path,
            1,
            &rule.id,
            format!(
                "documented rules without a configuration entry: {}",
                unconfigured.join(", ")
            ),
        ));
    }
    // The version contract: the document's `Version:` metadata line must equal the
    // configuration's standard.version.
    if !config.standard_version.is_empty() {
        match declared_version(text) {
            Some(version) if version == config.standard_version => {}
            Some(version) => findings.push(Finding::new(
                rule.severity,
                doc_path,
                1,
                &rule.id,
                format!(
                    "document {} version {} does not equal the configured standard.version {}",
                    doc_path, version, config.standard_version
                ),
            )),
            None => findings.push(Finding::new(
                rule.severity,
                doc_path,
                1,
                &rule.id,
                format!("document {} has no 'Version:' line", doc_path),
            )),
        }
    }
    if let Some(version_pattern) = rule.version_pattern.as_deref() {
        let found = Regex::new(version_pattern, false)
            .map(|re| text.lines().any(|line| re.is_match(line)))
            .unwrap_or(false);
        if !found {
            findings.push(Finding::new(
                rule.severity,
                doc_path,
                1,
                &rule.id,
                format!(
                    "document {} has no line matching the version pattern",
                    doc_path
                ),
            ));
        }
    }
    findings
}

/// Extracts the version from the document's first `Version:` metadata line, trimming
/// markdown emphasis and surrounding whitespace: `Version: **0.3**` yields `0.3`.
fn declared_version(text: &str) -> Option<String> {
    for line in text.lines() {
        let trimmed = line.trim();
        let trimmed = trimmed
            .strip_prefix('#')
            .map(str::trim_start)
            .unwrap_or(trimmed);
        if let Some(rest) = trimmed.strip_prefix("Version:") {
            let value = rest.trim().trim_matches('*').trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

/// CEP:WHAT: Locates the repository root for the CLI the same way the engine does: the
///   nearest ancestor (from the working directory) containing a `.cep` directory.
/// CEP:WHY: The --self-test flag must find the shipped manifest without a config flag.
/// CEP:STATUS: complete
/// CEP:FAILURE: Err outside a repository (mapped to the bootstrap exit code).
/// CEP:ASSUMES: same discovery contract as LintEngine::load.
/// CEP:COST: O(depth) stat calls.
/// CEP:EVIDENCE: integration::self_test_runs_the_shipped_manifest drives it end to end.
pub fn find_repo_root() -> Result<PathBuf, LintError> {
    discover_root(None)
}

/// CEP:WHAT: The --version banner: the tool version plus the enforced standard identity
///   from the effective configuration (tool version alone when no repository is found).
/// CEP:WHY: Section 50.1 requires --version to print tool AND standard versions, so a
///   run's enforcement target is visible in logs.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; falls back to the tool-only banner outside a repository.
/// CEP:ASSUMES: the default configuration names the standard the run would enforce.
/// CEP:COST: one config load.
/// CEP:EVIDENCE: integration::version_banner_names_the_standard.
pub fn version_banner(config_path: Option<&Path>) -> String {
    match LintEngine::load(config_path) {
        Ok(engine) => format!(
            "cep_lint {} (enforcing {} {})",
            crate::VERSION,
            engine.config().standard_name,
            engine.config().standard_version
        ),
        Err(_) => format!("cep_lint {} ({})", crate::VERSION, crate::SUMMARY),
    }
}

/// CEP:WHAT: Result of the self-test manifest run: the scenario count and one failure
///   string per failed scenario (empty means every scenario matched its exact counts).
/// CEP:WHY: The 50.6.3 gate needs a machine-checkable outcome the CLI maps to exit codes.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; a plain data carrier.
/// CEP:ASSUMES: scenarios ran in manifest order; failures carry the scenario name.
/// CEP:COST: one word per scenario.
/// CEP:EVIDENCE: integration::self_test_runs_the_shipped_manifest and the failure-path
///   tests in engine::tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelfTestOutcome {
    pub scenarios: usize,
    pub failures: Vec<String>,
}

/// CEP:WHAT: The self-test manifest runner (50.6.3): every scenario writes its fixture
///   files into a fresh temporary root, lints them through the FULL engine (rules, doc
///   sync, waiver application), and requires the exact per-rule finding counts it
///   declares; undeclared rules must produce zero findings.
/// CEP:WHY: The tool contract requires `--self-test` to prove every configured check
///   primitive still detects what the standard says it detects, with exact counts.
/// CEP:STATUS: complete
/// CEP:FAILURE: Err for an unreadable or unparseable manifest (bootstrap); scenario
///   failures (config rejection, fixture write failure, count mismatch) are collected in
///   SelfTestOutcome.failures and mapped to the findings exit code.
/// CEP:ASSUMES: manifest schema: { scenarios: [ { name, config (JSON text), files
///   (name -> source), doc?: { path, text }, expect_findings: { RULE: exact count } } ] };
///   scenario file_globs must match their fixture names ("**/*" is simplest).
/// CEP:COST: one temporary directory per scenario, removed after the scenario.
/// CEP:EVIDENCE: tools/cep_lint/tests/manifest.json scenarios; integration and engine
///   tests drive the runner directly.
pub fn run_self_test(manifest_path: &Path) -> Result<SelfTestOutcome, LintError> {
    let text = std::fs::read_to_string(manifest_path).map_err(|e| {
        LintError::new(format!(
            "cannot read self-test manifest {}: {}",
            manifest_path.display(),
            e
        ))
    })?;
    let manifest = json::parse(&text).map_err(|e| {
        LintError::new(format!("self-test manifest is not valid JSON: {}", e.message))
    })?;
    let scenarios = manifest
        .get("scenarios")
        .and_then(|value| value.as_array())
        .ok_or_else(|| LintError::new("self-test manifest has no scenarios array"))?;
    let mut outcome = SelfTestOutcome {
        scenarios: scenarios.len(),
        failures: Vec::new(),
    };
    for (index, scenario) in scenarios.iter().enumerate() {
        let name = scenario
            .get("name")
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .to_string();
        if let Err(detail) = run_scenario(index, scenario) {
            outcome
                .failures
                .push(format!("scenario '{}' ({})", name, detail));
        }
    }
    Ok(outcome)
}

/// Runs one self-test scenario; the error string is the failure detail.
fn run_scenario(index: usize, scenario: &json::Value) -> Result<(), String> {
    let config_text = scenario
        .get("config")
        .and_then(|value| value.as_str())
        .ok_or_else(|| "scenario has no config text".to_string())?;
    let config =
        Config::from_str(config_text).map_err(|e| format!("config rejected: {}", e.message))?;
    let files = match scenario.get("files") {
        Some(json::Value::Object(members)) => members,
        _ => return Err("scenario has no files object".to_string()),
    };
    if files.is_empty() {
        return Err("scenario has no fixture files".to_string());
    }
    let root = std::env::temp_dir().join(format!(
        "cep_lint_selftest_{}_{}",
        std::process::id(),
        index
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root)
        .map_err(|e| format!("cannot create temporary root {}: {}", root.display(), e))?;
    let mut result = Ok(());
    {
        for (name, content) in files {
            let body = match content.as_str() {
                Some(body) => body,
                None => {
                    result = Err(format!("fixture {} is not a string", name));
                    break;
                }
            };
            let path = root.join(name);
            if let Some(parent) = path.parent() {
                if std::fs::create_dir_all(parent).is_err() {
                    result = Err(format!("cannot create parent of fixture {}", name));
                    break;
                }
            }
            if let Err(e) = std::fs::write(&path, body) {
                result = Err(format!("cannot write fixture {}: {}", name, e));
                break;
            }
        }
        if result.is_ok() {
            if let Some(doc) = scenario.get("doc") {
                let doc_path = doc.get("path").and_then(|value| value.as_str()).unwrap_or("");
                let doc_text = doc.get("text").and_then(|value| value.as_str()).unwrap_or("");
                if doc_path.is_empty()
                    || std::fs::write(root.join(doc_path), doc_text).is_err()
                {
                    result = Err("scenario doc fixture is incomplete".to_string());
                }
            }
        }
    }
    if result.is_ok() {
        match LintEngine::with_parts(config, root.clone()).run(std::slice::from_ref(&root)) {
            Ok(outcome) => {
                let mut actual: BTreeMap<&str, usize> = BTreeMap::new();
                for finding in &outcome.findings {
                    *actual.entry(finding.rule.as_str()).or_insert(0) += 1;
                }
                let expected = match scenario.get("expect_findings") {
                    Some(json::Value::Object(members)) => members,
                    _ => return finish_scenario(root, Err("scenario has no expect_findings object".to_string())),
                };
                for (rule_id, count) in expected {
                    let want = match count.as_u64() {
                        Some(want) => want as usize,
                        None => {
                            result = Err(format!("expected count for {} is not an integer", rule_id));
                            break;
                        }
                    };
                    let got = actual.get(rule_id.as_str()).copied().unwrap_or(0);
                    if got != want {
                        result = Err(format!(
                            "{} expected {} finding(s), got {} (actual findings: {:?})",
                            rule_id, want, got, actual
                        ));
                        break;
                    }
                }
                if result.is_ok() {
                    for (rule_id, count) in &actual {
                        if !expected.iter().any(|(name, _)| name == rule_id) {
                            result = Err(format!(
                                "unexpected findings from {} ({})",
                                rule_id, count
                            ));
                            break;
                        }
                    }
                }
            }
            Err(e) => result = Err(format!("engine run failed: {}", e.message)),
        }
    }
    finish_scenario(root, result)
}

/// Removes the scenario root and passes the result through.
fn finish_scenario(root: PathBuf, result: Result<(), String>) -> Result<(), String> {
    let _ = std::fs::remove_dir_all(&root);
    result
}

/// CEP:WHAT: Free-function entry point from the tool contract: lint paths against a config
///   without waivers or repository-root discovery, reporting paths verbatim.
/// CEP:WHY: Library consumers and tests need pure linting with no environment coupling.
/// CEP:STATUS: complete
/// CEP:FAILURE: unreadable files are skipped silently (bootstrap errors are the engine's
///   run() contract); oversized files yield CEP-LINT-BUDGET findings.
/// CEP:ASSUMES: callers accept verbatim path reporting and no waiver application.
/// CEP:COST: O(input bytes); iterative directory walk with sorted reads.
/// CEP:EVIDENCE: integration::same_input_yields_identical_findings_twice,
///   oversized_file_yields_budget_finding_not_blowup, and key_vocabulary_rule_flags_
///   unknown_keys drive this entry point.
pub fn lint_paths(paths: &[PathBuf], config: &Config) -> Vec<Finding> {
    let mut findings = Vec::new();
    let mut files: Vec<PathBuf> = Vec::new();
    for path in paths {
        collect_simple(path, &mut files);
    }
    files.sort();
    for file in &files {
        let meta = match std::fs::metadata(file) {
            Ok(meta) => meta,
            Err(_) => continue,
        };
        let rel = file.to_string_lossy().replace('\\', "/");
        if meta.len() > MAX_FILE_BYTES {
            findings.push(Finding::new(
                1,
                rel.as_str(),
                1,
                BUDGET_RULE,
                format!(
                    "file is {} bytes, above the {} scan budget; not linted",
                    meta.len(),
                    MAX_FILE_BYTES
                ),
            ));
            continue;
        }
        let source = match std::fs::read_to_string(file) {
            Ok(source) => source,
            Err(_) => continue,
        };
        let scanned = scan::scan(&source);
        findings.extend(rules::run_file_rules(config, rel.as_str(), &scanned));
    }
    for rule in config.rules_with_check("doc_rule_coverage") {
        if let Some(doc_path) = rule.doc_path.as_deref() {
            if let Ok(text) = std::fs::read_to_string(doc_path) {
                findings.extend(doc_sync(rule, config, doc_path, &text));
            }
        }
    }
    findings.sort();
    findings
}

fn collect_simple(path: &Path, files: &mut Vec<PathBuf>) {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(_) => return,
    };
    if meta.is_symlink() {
        return;
    }
    if meta.is_file() {
        files.push(path.to_path_buf());
        return;
    }
    if !meta.is_dir() {
        return;
    }
    let mut stack: Vec<PathBuf> = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        let mut names: Vec<PathBuf> = entries
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .collect();
        names.sort();
        for entry in names {
            let meta = match std::fs::symlink_metadata(&entry) {
                Ok(meta) => meta,
                Err(_) => continue,
            };
            if meta.is_symlink() {
                continue;
            }
            if meta.is_dir() {
                let base = entry.file_name().and_then(|n| n.to_str()).unwrap_or_default();
                if base != "target" && base != ".git" {
                    stack.push(entry);
                }
            } else if meta.is_file() {
                files.push(entry);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
    }

    #[test]
    fn doc_sync_passes_the_shipped_document_and_config() {
        let root = manifest_root();
        let config = Config::from_file(&root.join(".cep").join("cep_lint.json")).expect("config");
        let rule = config
            .rules_with_check("doc_rule_coverage")
            .into_iter()
            .next()
            .expect("doc rule");
        let text = std::fs::read_to_string(root.join("standard.md")).expect("standard.md");
        let findings = doc_sync(rule, &config, "standard.md", &text);
        assert!(
            findings.is_empty(),
            "shipped doc/config must stay in sync: {:?}",
            findings
        );
    }

    #[test]
    fn doc_sync_flags_missing_heading_and_version_mismatch() {
        let root = manifest_root();
        let config = Config::from_file(&root.join(".cep").join("cep_lint.json")).expect("config");
        let rule = config
            .rules_with_check("doc_rule_coverage")
            .into_iter()
            .next()
            .expect("doc rule");
        let findings = doc_sync(rule, &config, "doc.md", "# small doc\nVersion: 9.9\n");
        assert!(!findings.is_empty());
        assert!(findings.iter().any(|f| f.message.contains("without a '### Rule' heading")));
        assert!(findings
            .iter()
            .any(|f| f.message.contains("does not equal the configured standard.version")));
    }

    /// Synthetic doc-sync config: one trivial code rule plus the doc-sync rule.
    fn config_with_doc_sync(version: &str, version_pattern: Option<&str>) -> Config {
        let pattern_part = match version_pattern {
            Some(p) => format!(", \"version_pattern\": \"{}\"", p),
            None => String::new(),
        };
        let text = format!(
            "{{\"standard\": {{\"name\": \"CEP&CC\", \"version\": \"{}\", \"source\": \"standard.md\"}}, \
             \"file_globs\": [\"**/*\"], \"rules\": {{\
             \"CEP-LINT-A\": {{\"severity\": 2, \"check\": \"comment_regex\", \"pattern\": \"\\\\bTODO\\\\b\"}}, \
             \"CEP-LINT-DOC-SYNC\": {{\"severity\": 1, \"check\": \"doc_rule_coverage\", \
             \"doc_path\": \"doc.md\", \"heading_pattern\": \"^### Rule (CEP-LINT-[A-Z-]+)$\"{}}}}}}}",
            version, pattern_part
        );
        Config::from_str(&text).expect("synthetic config")
    }

    const AGREEMENT_DOC: &str = "\
# T
Version: **0.2**
### Rule CEP-LINT-A
### Rule CEP-LINT-DOC-SYNC
";

    #[test]
    fn doc_sync_full_agreement_is_clean() {
        let config = config_with_doc_sync("0.2", None);
        let rule = config
            .rules_with_check("doc_rule_coverage")
            .into_iter()
            .next()
            .expect("doc rule");
        assert!(doc_sync(rule, &config, "doc.md", AGREEMENT_DOC).is_empty());
    }

    #[test]
    fn doc_sync_reverse_direction_flags_documented_rules_without_configuration() {
        let config = config_with_doc_sync("0.2", None);
        let rule = config
            .rules_with_check("doc_rule_coverage")
            .into_iter()
            .next()
            .expect("doc rule");
        let text = AGREEMENT_DOC.replace(
            "### Rule CEP-LINT-DOC-SYNC",
            "### Rule CEP-LINT-DOC-SYNC\n### Rule CEP-LINT-B",
        );
        let findings = doc_sync(rule, &config, "doc.md", &text);
        assert!(findings
            .iter()
            .any(|f| f.message.contains("documented rules without a configuration entry")
                && f.message.contains("CEP-LINT-B")),
            "{:?}",
            findings
        );
    }

    #[test]
    fn doc_sync_version_equality_ignores_stale_cross_references() {
        // The document carries a stale narrative mention of an old version; the version
        // contract reads the `Version:` line, so agreement still holds.  This is the
        // regression for the 0.1/0.2 drift: an any-line pattern search used to be
        // satisfied by exactly such a mention while the document had moved on.
        let config = config_with_doc_sync("0.2", None);
        let rule = config
            .rules_with_check("doc_rule_coverage")
            .into_iter()
            .next()
            .expect("doc rule");
        let text = AGREEMENT_DOC
            .replace("### Rule CEP-LINT-A", "narrative: CEP&CC 0.1 introduced chapters.\n### Rule CEP-LINT-A");
        let findings = doc_sync(rule, &config, "doc.md", &text);
        assert!(
            !findings.iter().any(|f| f.message.contains("does not equal")),
            "{:?}",
            findings
        );
    }

    #[test]
    fn doc_sync_flags_version_mismatch_despite_stale_reference() {
        let config = config_with_doc_sync("0.3", None);
        let rule = config
            .rules_with_check("doc_rule_coverage")
            .into_iter()
            .next()
            .expect("doc rule");
        let text = AGREEMENT_DOC
            .replace("### Rule CEP-LINT-A", "narrative: CEP&CC 0.1 introduced chapters.\n### Rule CEP-LINT-A");
        let findings = doc_sync(rule, &config, "doc.md", &text);
        assert!(findings
            .iter()
            .any(|f| f.message.contains("version 0.2 does not equal the configured standard.version 0.3")),
            "{:?}",
            findings
        );
    }

    #[test]
    fn doc_sync_version_pattern_remains_supported_when_configured() {
        let config = config_with_doc_sync("0.2", Some("CEP&CC 0\\\\.1"));
        let rule = config
            .rules_with_check("doc_rule_coverage")
            .into_iter()
            .next()
            .expect("doc rule");
        // The stale mention satisfies the optional pattern; no pattern finding appears.
        let text = AGREEMENT_DOC
            .replace("### Rule CEP-LINT-A", "narrative: CEP&CC 0.1 introduced chapters.\n### Rule CEP-LINT-A");
        let findings = doc_sync(rule, &config, "doc.md", &text);
        assert!(
            !findings.iter().any(|f| f.message.contains("version pattern")),
            "{:?}",
            findings
        );
        // Without any matching line the optional pattern fires on top of the equality check.
        let findings = doc_sync(rule, &config, "doc.md", AGREEMENT_DOC);
        assert!(findings
            .iter()
            .any(|f| f.message.contains("no line matching the version pattern")),
            "{:?}",
            findings
        );
    }

    #[test]
    fn declared_version_trims_markdown_emphasis() {
        assert_eq!(declared_version("# T\nVersion: **0.3**\n"), Some("0.3".to_string()));
        assert_eq!(declared_version("Version: 0.3\n"), Some("0.3".to_string()));
        assert_eq!(declared_version("# Version: **0.4**\n"), Some("0.4".to_string()));
        assert_eq!(declared_version("# no version here\n"), None);
    }

    #[test]
    fn self_test_runner_accepts_exact_counts_and_flags_mismatches() {
        let dir = std::env::temp_dir().join(format!(
            "cep_lint_manifest_engine_test_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let scenario_config = "{\"standard\":{\"name\":\"CEP&CC\",\"version\":\"0.3\",\"source\":\"x\"},\
             \"file_globs\":[\"**/*\",\"*\"],\
             \"rules\":{\"CEP-LINT-TODO\":{\"severity\":1,\"check\":\"comment_regex\",\
             \"pattern\":\"\\\\b(TODO|FIXME)\\\\b\"}}}";
        let escaped = scenario_config.replace('\\', "\\\\").replace('"', "\\\"");
        let manifest = format!(
            "{{\"scenarios\": [\
             {{\"name\": \"todo_found\", \"config\": \"{cfg}\", \
             \"files\": {{\"a.rs\": \"// TODO fix\\n\"}}, \
             \"expect_findings\": {{\"CEP-LINT-TODO\": 1}}}}, \
             {{\"name\": \"bad_count\", \"config\": \"{cfg}\", \
             \"files\": {{\"a.rs\": \"// TODO fix\\n\"}}, \
             \"expect_findings\": {{\"CEP-LINT-TODO\": 2}}}}]}}",
            cfg = escaped
        );
        let path = dir.join("manifest.json");
        std::fs::write(&path, &manifest).expect("manifest");
        let outcome = run_self_test(&path).expect("manifest runs");
        assert_eq!(outcome.scenarios, 2);
        assert_eq!(outcome.failures.len(), 1, "{:?}", outcome.failures);
        assert!(outcome.failures[0].contains("bad_count"), "{:?}", outcome.failures);
        assert!(outcome.failures[0].contains("expected 2 finding(s), got 1"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
