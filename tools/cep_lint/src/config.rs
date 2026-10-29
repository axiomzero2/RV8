// CEP:FILE: tools/cep_lint/src/config.rs
// CEP:WHAT: The lint configuration model: fail-on severities, exit codes, file globs, and the
//   rule table parsed from .cep/cep_lint.json, plus the path-glob matcher used for scope and
//   waiver matching.
// CEP:WHY: Rule identities, severities, and parameters live in configuration, not code, so
//   the engine stays generic and the standard document and config can be kept in sync.
// CEP:CLASS: CEP-2
// CEP:STATUS: complete
// CEP:OWNER: axiomzero2
// CEP:TICKET: RV8-100
// CEP:FAILURE: missing required keys, wrong-typed values, and unknown severity semantics
//   return descriptive Err values (bootstrap class); no default silently repairs the config.
// CEP:ASSUMES: the configuration file is first-party and follows the documented JSON shape;
//   unknown per-rule keys are tolerated so the schema can grow without breaking the tool.
// CEP:COST: O(config size) to parse and validate; rule lookup by check/mode is O(R).
// CEP:EVIDENCE: unit tests parse a faithful copy of the shipped config and assert each
//   parameter, plus type-error and missing-key error paths.
// CEP:SECURITY: glob matching is iterative with bounded state; untrusted globs cannot drive
//   recursion (38.13).

use crate::json::{self, Value};
use std::str::FromStr;

/// CEP:WHAT: One configured lint rule with every parameter the engine understands; rule
///   identity, severity, and policy all live here instead of in engine code.
/// CEP:WHY: The standard demands config-driven enforcement, so adding or retuning a rule is
///   a data change, never a tool change; this struct is that data's typed shape.
/// CEP:STATUS: complete
/// CEP:FAILURE: parse_rule returns a descriptive ConfigError for missing or mistyped keys;
///   no field is silently defaulted except documented severities and position limits.
/// CEP:ASSUMES: the shipped .cep/cep_lint.json shape; unknown per-rule keys are tolerated
///   so the schema can grow without breaking older tools.
/// CEP:COST: O(1) per lookup helper; the struct is cloned only at engine setup.
/// CEP:EVIDENCE: config::tests::parses_shipped_config asserts every parameter against the
///   shipped configuration, plus type-error and missing-key error paths.
#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    /// Config key, for example "CEP-LINT-FILE-HEADER".
    pub id: String,
    pub severity: u8,
    /// Dispatch key, for example "comment_block_schema".
    pub check: String,
    /// Variant within a check, for example "file_header".
    pub mode: Option<String>,
    pub required_fields: Vec<String>,
    pub pattern: Option<String>,
    pub suppression_pattern: Option<String>,
    pub icase: bool,
    pub name_pattern: Option<String>,
    /// Vocabulary for status values and allowed-literal checks.
    pub allowed_values: Vec<String>,
    pub statuses: Vec<String>,
    pub suppress_statuses: Vec<String>,
    pub status_field: String,
    pub class_field: String,
    pub allowed_classes: Vec<String>,
    pub applies_to_classes: Vec<String>,
    pub tokens: Vec<String>,
    /// Numeric allow-list kept in source form for exact numeric comparison.
    pub allowed_numbers: Vec<String>,
    pub exempt_contexts: Vec<String>,
    pub min_body_statements: u64,
    pub max_start_line: u64,
    pub doc_path: Option<String>,
    pub heading_pattern: Option<String>,
    pub version_pattern: Option<String>,
    /// Key vocabulary for the optional key-vocabulary check (absent in the shipped config).
    pub allowed_fields: Option<Vec<String>>,
    /// Explicit activation switch for rules whose mechanical scoping is policy-dependent
    /// (the banned-token rule is advisory until the configuration sets this).
    pub enforce: bool,
}

impl Rule {
    /// Severity of findings produced by this rule.
    pub fn severity_of(&self) -> u8 {
        self.severity
    }
}

/// CEP:WHAT: The exit-code table mapped from the configuration's exit_codes object.
/// CEP:WHY: CI reads meaning from exit codes, so clean/findings/bootstrap/usage must come
///   from policy, not be hard-coded in the binary.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; Default supplies the shipped 0/1/2/3 contract when absent.
/// CEP:ASSUMES: configured codes fit the process exit-code range (validated at parse).
/// CEP:COST: Copy type; zero allocation.
/// CEP:EVIDENCE: config::tests::parses_shipped_config pins all four values; integration
///   tests assert each code end to end through the binary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExitCodes {
    pub clean: i32,
    pub findings: i32,
    pub bootstrap: i32,
    pub usage: i32,
}

impl Default for ExitCodes {
    fn default() -> ExitCodes {
        ExitCodes {
            clean: 0,
            findings: 1,
            bootstrap: 2,
            usage: 3,
        }
    }
}

/// CEP:WHAT: The parsed lint configuration: standard identity, fail-on severities, exit
///   codes, scope globs, and the rule table.
/// CEP:WHY: One immutable structure is shared by the CLI, the library API, and the tests so
///   every consumer enforces exactly the same policy.
/// CEP:STATUS: complete
/// CEP:FAILURE: from_str/from_file return ConfigError for invalid JSON, a missing rules
///   table, mistyped keys, or uncompilable patterns; no default silently repairs input.
/// CEP:ASSUMES: the configuration is the first-party .cep/cep_lint.json shape.
/// CEP:COST: O(config size) to parse and validate; rule lookup is O(R).
/// CEP:EVIDENCE: config::tests parse the shipped file, malformed inputs, and glob
///   semantics; engine tests consume the parsed form end to end.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub standard_name: String,
    pub standard_version: String,
    /// Severities that fail the run.
    pub fail_on: Vec<u8>,
    pub exit_codes: ExitCodes,
    /// Path globs (repo-relative, forward slashes) selecting in-scope files.
    pub file_globs: Vec<String>,
    pub rules: Vec<Rule>,
}

/// CEP:WHAT: Configuration load or validation failure, carrying a human-readable message.
/// CEP:WHY: Bootstrap failures must be distinguishable from findings and reportable to a
///   human without a panic path.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; constructing an error cannot fail.
/// CEP:ASSUMES: messages are first-party diagnostics, not parsed by machines.
/// CEP:COST: one String per error, only on the failure path.
/// CEP:EVIDENCE: config::tests::missing_and_mistyped_keys_are_err asserts the message
///   content and that malformed inputs return Err rather than panicking.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfigError {
    pub message: String,
}

impl ConfigError {
    fn new(message: impl Into<String>) -> ConfigError {
        ConfigError {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ConfigError {}

impl Default for Config {
    fn default() -> Config {
        Config {
            standard_name: String::new(),
            standard_version: String::new(),
            fail_on: vec![0, 1],
            exit_codes: ExitCodes::default(),
            file_globs: Vec::new(),
            rules: Vec::new(),
        }
    }
}

fn type_error(context: &str, key: &str, found: &str, expected: &str) -> ConfigError {
    ConfigError::new(format!(
        "configuration key '{}' in {} has type {}, expected {}",
        key, context, found, expected
    ))
}

fn get_u8(value: &Value, context: &str, key: &str) -> Result<Option<u8>, ConfigError> {
    match value.get(key) {
        None => Ok(None),
        Some(v) => match v.as_u64() {
            Some(n) if n <= u8::MAX as u64 => Ok(Some(n as u8)),
            _ => Err(type_error(context, key, v.kind(), "integer 0..255")),
        },
    }
}

fn get_u64(value: &Value, context: &str, key: &str) -> Result<Option<u64>, ConfigError> {
    match value.get(key) {
        None => Ok(None),
        Some(v) => match v.as_u64() {
            Some(n) => Ok(Some(n)),
            None => Err(type_error(context, key, v.kind(), "non-negative integer")),
        },
    }
}

fn get_bool(value: &Value, context: &str, key: &str) -> Result<Option<bool>, ConfigError> {
    match value.get(key) {
        None => Ok(None),
        Some(v) => match v.as_bool() {
            Some(b) => Ok(Some(b)),
            None => Err(type_error(context, key, v.kind(), "boolean")),
        },
    }
}

fn get_string(value: &Value, context: &str, key: &str) -> Result<Option<String>, ConfigError> {
    match value.get(key) {
        None => Ok(None),
        Some(v) => match v.as_str() {
            Some(s) => Ok(Some(s.to_string())),
            None => Err(type_error(context, key, v.kind(), "string")),
        },
    }
}

fn get_string_vec(value: &Value, context: &str, key: &str) -> Result<Vec<String>, ConfigError> {
    match value.get(key) {
        None => Ok(Vec::new()),
        Some(v) => {
            let items = match v.as_array() {
                Some(items) => items,
                None => return Err(type_error(context, key, v.kind(), "array of strings")),
            };
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                match item.as_str() {
                    Some(s) => out.push(s.to_string()),
                    None => return Err(type_error(context, key, v.kind(), "array of strings")),
                }
            }
            Ok(out)
        }
    }
}

fn get_opt_string_vec(
    value: &Value,
    context: &str,
    key: &str,
) -> Result<Option<Vec<String>>, ConfigError> {
    match value.get(key) {
        None => Ok(None),
        Some(_) => Ok(Some(get_string_vec(value, context, key)?)),
    }
}

impl Config {
    /// CEP:WHAT: Parses configuration text (JSON) into a validated Config.
    /// CEP:WHY: The single entry point from raw bytes to enforced policy; also bound as the
    ///   std::str::FromStr impl so callers can use the conventional idiom.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: Err for invalid JSON, a missing rules table, mistyped keys, or any
    ///   regex the embedded engine cannot compile; never panics on any input.
    /// CEP:ASSUMES: source is valid UTF-8 text in the documented shape.
    /// CEP:COST: O(config bytes) for the parse plus O(R * patterns) for validation.
    /// CEP:EVIDENCE: config::tests cover the shipped document, malformed JSON, mistyped
    ///   keys, and unsupported patterns.
    pub fn parse(source: &str) -> Result<Config, ConfigError> {
        let root = json::parse(source)
            .map_err(|e| ConfigError::new(format!("configuration is not valid JSON: {}", e)))?;
        if root.get("rules").is_none() {
            return Err(ConfigError::new("configuration is missing required key 'rules'"));
        }
        let mut config = Config::default();
        if let Some(standard) = root.get("standard") {
            config.standard_name = get_string(standard, "standard", "name")?
                .unwrap_or_default();
            config.standard_version = get_string(standard, "standard", "version")?
                .unwrap_or_default();
        }
        if let Some(fail_on) = root.get("fail_on") {
            let items = match fail_on.as_array() {
                Some(items) => items,
                None => return Err(type_error("configuration", "fail_on", fail_on.kind(), "array")),
            };
            let mut severities = Vec::with_capacity(items.len());
            for item in items {
                match item.as_u64() {
                    Some(n) if n <= u8::MAX as u64 => severities.push(n as u8),
                    _ => {
                        return Err(type_error(
                            "configuration",
                            "fail_on",
                            item.kind(),
                            "integer 0..255",
                        ));
                    }
                }
            }
            config.fail_on = severities;
        }
        if let Some(exit_codes) = root.get("exit_codes") {
            let clean = get_u64(exit_codes, "exit_codes", "clean")?;
            let findings = get_u64(exit_codes, "exit_codes", "findings")?;
            let bootstrap = get_u64(exit_codes, "exit_codes", "bootstrap")?;
            let usage = get_u64(exit_codes, "exit_codes", "usage")?;
            let to_i32 = |v: Option<u64>, key: &str| -> Result<i32, ConfigError> {
                match v {
                    Some(n) if n <= i32::MAX as u64 => Ok(n as i32),
                    Some(_) => Err(ConfigError::new(format!(
                        "exit_codes.{} exceeds the process exit-code range",
                        key
                    ))),
                    None => Err(ConfigError::new(format!(
                        "exit_codes is present but '{}' is missing",
                        key
                    ))),
                }
            };
            config.exit_codes = ExitCodes {
                clean: to_i32(clean, "clean")?,
                findings: to_i32(findings, "findings")?,
                bootstrap: to_i32(bootstrap, "bootstrap")?,
                usage: to_i32(usage, "usage")?,
            };
        }
        config.file_globs = get_string_vec(&root, "configuration", "file_globs")?;
        let rules_value = match root.get("rules") {
            Some(value) => value,
            // Unreachable after the presence check above, but the library never panics.
            None => return Err(ConfigError::new("configuration is missing required key 'rules'")),
        };
        let members = match rules_value.as_array() {
            None => match rules_value {
                Value::Object(members) => members,
                other => {
                    return Err(type_error("configuration", "rules", other.kind(), "object"));
                }
            },
            Some(_) => {
                return Err(type_error(
                    "configuration",
                    "rules",
                    rules_value.kind(),
                    "object",
                ));
            }
        };
        let mut rules = Vec::with_capacity(members.len());
        for (id, body) in members {
            rules.push(Self::parse_rule(id, body)?);
        }
        config.rules = rules;
        Self::validate_patterns(&config)?;
        Ok(config)
    }

    /// Every configured regex must compile at load time; a bad pattern is a bootstrap
    /// error rather than a silent mid-run skip.
    fn validate_patterns(config: &Config) -> Result<(), ConfigError> {
        for rule in &config.rules {
            let check = |key: &str, pattern: Option<&str>, icase: bool| -> Result<(), ConfigError> {
                if let Some(pattern) = pattern {
                    if let Err(e) = crate::regexlite::Regex::new(pattern, icase) {
                        return Err(ConfigError::new(format!(
                            "rules.{}: {} is not a supported pattern: {}",
                            rule.id, key, e.message
                        )));
                    }
                }
                Ok(())
            };
            check("pattern", rule.pattern.as_deref(), rule.icase)?;
            check("suppression_pattern", rule.suppression_pattern.as_deref(), rule.icase)?;
            check("name_pattern", rule.name_pattern.as_deref(), false)?;
            check("heading_pattern", rule.heading_pattern.as_deref(), rule.icase)?;
            check("version_pattern", rule.version_pattern.as_deref(), false)?;
        }
        Ok(())
    }

    /// CEP:WHAT: Loads and parses a configuration file from disk.
    /// CEP:WHY: The engine bootstrap needs one call that turns a path into policy and
    ///   names the offending file in the error.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: Err when the file cannot be read or the content fails parse; the path
    ///   appears in the message. Never panics.
    /// CEP:ASSUMES: the file is UTF-8.
    /// CEP:COST: one read plus the parse.
    /// CEP:EVIDENCE: LintEngine::load exercises this in the integration suite; the CI gate
    ///   loads the shipped file on every run.
    pub fn from_file(path: &std::path::Path) -> Result<Config, ConfigError> {
        let source = std::fs::read_to_string(path).map_err(|e| {
            ConfigError::new(format!(
                "cannot read configuration {}: {}",
                path.display(),
                e
            ))
        })?;
        Self::parse(&source)
    }

    fn parse_rule(id: &str, body: &Value) -> Result<Rule, ConfigError> {
        let context = format!("rules.{}", id);
        if body.get("check").is_none() {
            return Err(ConfigError::new(format!("{} is missing 'check'", context)));
        }
        // allowed_values carries strings for vocabularies and numbers for literal
        // allow-lists; they are collected separately so each rule reads its own form.
        let mut allowed_numbers = Vec::new();
        let mut allowed_values = Vec::new();
        if let Some(list) = body.get("allowed_values") {
            match list {
                Value::Array(items) => {
                    for item in items {
                        match item {
                            Value::Number(text) => allowed_numbers.push(text.clone()),
                            Value::String(text) => allowed_values.push(text.clone()),
                            other => {
                                return Err(type_error(
                                    &context,
                                    "allowed_values",
                                    other.kind(),
                                    "number or string",
                                ));
                            }
                        }
                    }
                }
                other => {
                    return Err(type_error(&context, "allowed_values", other.kind(), "array"));
                }
            }
        }
        Ok(Rule {
            id: id.to_string(),
            severity: get_u8(body, &context, "severity")?.unwrap_or(2),
            check: get_string(body, &context, "check")?
                .ok_or_else(|| ConfigError::new(format!("{} has empty 'check'", context)))?,
            mode: get_string(body, &context, "mode")?,
            required_fields: get_string_vec(body, &context, "required_fields")?,
            pattern: get_string(body, &context, "pattern")?,
            suppression_pattern: get_string(body, &context, "suppression_pattern")?,
            icase: get_bool(body, &context, "icase")?.unwrap_or(false),
            name_pattern: get_string(body, &context, "name_pattern")?,
            allowed_values,
            statuses: get_string_vec(body, &context, "statuses")?,
            suppress_statuses: get_string_vec(body, &context, "suppress_statuses")?,
            status_field: get_string(body, &context, "status_field")?
                .unwrap_or_else(|| "CEP:STATUS".to_string()),
            class_field: get_string(body, &context, "class_field")?
                .unwrap_or_else(|| "CEP:CLASS".to_string()),
            allowed_classes: get_string_vec(body, &context, "allowed_classes")?,
            applies_to_classes: get_string_vec(body, &context, "applies_to_classes")?,
            tokens: get_string_vec(body, &context, "tokens")?,
            allowed_numbers,
            exempt_contexts: get_string_vec(body, &context, "exempt_contexts")?,
            min_body_statements: get_u64(body, &context, "min_body_statements")?.unwrap_or(2),
            max_start_line: get_u64(body, &context, "max_start_line")?.unwrap_or(1),
            doc_path: get_string(body, &context, "doc_path")?,
            heading_pattern: get_string(body, &context, "heading_pattern")?,
            version_pattern: get_string(body, &context, "version_pattern")?,
            allowed_fields: get_opt_string_vec(body, &context, "allowed_fields")?,
            enforce: get_bool(body, &context, "enforce")?.unwrap_or(false),
        })
    }

    /// Finds the rule implementing a (check, mode) pair, if configured.
    pub fn rule_by_check(&self, check: &str, mode: Option<&str>) -> Option<&Rule> {
        self.rules.iter().find(|r| {
            r.check == check
                && match mode {
                    None => true,
                    Some(m) => r.mode.as_deref() == Some(m),
                }
        })
    }

    /// All rules whose check kind matches (mode-agnostic).
    pub fn rules_with_check(&self, check: &str) -> Vec<&Rule> {
        self.rules.iter().filter(|r| r.check == check).collect()
    }

    /// Whether a severity fails the run.
    pub fn fails(&self, severity: u8) -> bool {
        self.fail_on.contains(&severity)
    }
}

impl FromStr for Config {
    type Err = ConfigError;

    fn from_str(source: &str) -> Result<Config, ConfigError> {
        Config::parse(source)
    }
}

/// CEP:WHAT: Reports whether `path` (forward-slash separated) matches `pattern` with `**`
///   spanning directory levels and `*`/`?` staying within one segment.
/// CEP:WHY: Scope selection (file_globs) and waiver SCOPE lines both need deterministic
///   glob semantics without an external crate.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; unmatched patterns simply return false.
/// CEP:ASSUMES: patterns and paths use `/` separators; a leading `**/` and trailing `/**`
///   both match zero directories (corpus-tested).
/// CEP:COST: iterative two-pointer match with an explicit stack; bounded by pattern and
///   path segment counts, never by backtracking blowup.
/// CEP:EVIDENCE: config::tests::glob_semantics covers spanning, segment boundaries,
///   `?` classes, and multi-pattern dispatch.
pub fn glob_match(pattern: &str, path: &str) -> bool {
    let pat: Vec<&str> = pattern.split('/').collect();
    let segs: Vec<&str> = path.split('/').collect();
    let mut stack: Vec<(usize, usize)> = vec![(0, 0)];
    while let Some((mut pi, mut si)) = stack.pop() {
        loop {
            match pat.get(pi) {
                None => {
                    if si == segs.len() {
                        return true;
                    }
                    break;
                }
                Some(&"**") => {
                    if pi + 1 == pat.len() {
                        return true;
                    }
                    if si < segs.len() {
                        stack.push((pi, si + 1));
                    }
                    stack.push((pi + 1, si));
                    break;
                }
                Some(p) => match segs.get(si) {
                    None => break,
                    Some(s) => {
                        if !glob_segment(p, s) {
                            break;
                        }
                        pi += 1;
                        si += 1;
                    }
                },
            }
        }
    }
    false
}

/// Reports whether any configured glob matches the path.
pub fn any_glob_match(patterns: &[String], path: &str) -> bool {
    patterns.iter().any(|p| glob_match(p, path))
}

fn glob_segment(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let mut stack: Vec<(usize, usize)> = vec![(0, 0)];
    while let Some((mut pi, mut ti)) = stack.pop() {
        loop {
            match p.get(pi) {
                None => {
                    if ti == t.len() {
                        return true;
                    }
                    break;
                }
                Some('*') => {
                    stack.push((pi + 1, ti));
                    if ti < t.len() {
                        stack.push((pi, ti + 1));
                    }
                    break;
                }
                Some('?') => {
                    if ti < t.len() {
                        pi += 1;
                        ti += 1;
                    } else {
                        break;
                    }
                }
                Some(&c) => {
                    if ti < t.len() && t[ti] == c {
                        pi += 1;
                        ti += 1;
                    } else {
                        break;
                    }
                }
            }
        }
    }
    false
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The shipped configuration, parsed by several test modules.
    pub(crate) const SHIPPED_TEXT: &str = include_str!("../../../.cep/cep_lint.json");

    #[test]
    fn parses_shipped_config() {
        let config = Config::from_str(SHIPPED_TEXT).expect("shipped config parses");
        assert_eq!(config.standard_name, "CEP&CC");
        assert_eq!(config.standard_version, "0.1");
        assert_eq!(config.fail_on, vec![0, 1]);
        assert_eq!(config.exit_codes.clean, 0);
        assert_eq!(config.exit_codes.findings, 1);
        assert_eq!(config.exit_codes.bootstrap, 2);
        assert_eq!(config.exit_codes.usage, 3);
        assert_eq!(config.file_globs.len(), 3);
        assert!(config.file_globs.contains(&"crates/**/*.rs".to_string()));
        assert_eq!(config.rules.len(), 17);
        let header = config
            .rule_by_check("comment_block_schema", Some("file_header"))
            .expect("file header rule");
        assert_eq!(header.id, "CEP-LINT-FILE-HEADER");
        assert_eq!(header.severity, 1);
        assert_eq!(header.required_fields.len(), 9);
        assert!(header.required_fields.contains(&"CEP:FILE".to_string()));
        assert_eq!(header.max_start_line, 1);
        assert!(header.allowed_classes.contains(&"HPC-0".to_string()));
        let func = config
            .rule_by_check("function_policy", Some("block"))
            .expect("func block rule");
        assert_eq!(func.min_body_statements, 2);
        let hot = config
            .rule_by_check("banned_token_policy", None)
            .expect("hot rule");
        assert_eq!(hot.severity, 0);
        assert!(!hot.enforce, "shipped config leaves hot-token enforcement off");
        let doc = config
            .rule_by_check("doc_rule_coverage", None)
            .expect("doc sync rule");
        assert_eq!(doc.doc_path.as_deref(), Some("standard.md"));
    }

    #[test]
    fn malformed_json_is_err_not_panic() {
        assert!(Config::from_str("{").is_err());
        assert!(Config::from_str("not json").is_err());
        assert!(Config::from_str("").is_err());
    }

    #[test]
    fn missing_and_mistyped_keys_are_err() {
        assert!(Config::from_str("{}").is_err());
        let err = Config::from_str(r#"{"rules": {"A": {"severity": "high", "check": "c"}}}"#)
            .expect_err("bad severity type");
        assert!(err.message.contains("rules.A"));
        assert!(Config::from_str(r#"{"rules": [], "fail_on": [0]}"#).is_err());
        assert!(Config::from_str(r#"{"rules": {"A": {}}, "fail_on": "01"}"#).is_err());
    }

    #[test]
    fn glob_semantics() {
        assert!(glob_match("crates/**/*.rs", "crates/rv8_ir/src/hir.rs"));
        assert!(glob_match("tests/**/*.rs", "tests/fixtures/add_main.rs"));
        assert!(!glob_match("tests/**/*.rs", "tools/cep_lint/src/lib.rs"));
        assert!(glob_match("tools/**/*.rs", "tools/cep_lint/src/main.rs"));
        assert!(!glob_match("crates/**/*.rs", "quarantine/x.rs"));
        assert!(glob_match("**/*.rs", "a/b/c.rs"));
        assert!(glob_match("a/*.rs", "a/b.rs"));
        assert!(!glob_match("a/*.rs", "a/b/c.rs"));
        assert!(glob_match("a?c/*.rs", "abc/x.rs"));
        assert!(!glob_match("a?c/*.rs", "abbc/x.rs"));
        assert!(any_glob_match(
            &["crates/**/*.rs".to_string(), "tools/**/*.rs".to_string()],
            "tools/x.rs"
        ));
    }

    #[test]
    fn fails_uses_fail_on() {
        let config = Config::from_str(SHIPPED_TEXT).expect("parses");
        assert!(config.fails(0));
        assert!(config.fails(1));
        assert!(!config.fails(2));
    }

    #[test]
    fn unsupported_patterns_are_bootstrap_errors() {
        let src = r#"{"rules": {"A": {"check": "comment_regex", "severity": 1, "pattern": "("}}}"#;
        let err = Config::from_str(src).expect_err("bad pattern");
        assert!(err.message.contains("rules.A"));
    }
}
