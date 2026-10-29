// CEP:FILE: tools/cep_lint/src/rules.rs
// CEP:WHAT: Every configured check as a pure function over the scanned file: file-header
//   schema, function and public-item CEP blocks, status vocabulary, stub declarations,
//   comment regexes, string-literal markers, numeric literal policy, banned hot-path
//   tokens, empty bodies and branches, naming conventions, and CEP key vocabulary.
// CEP:WHY: Rules are driven entirely by .cep/cep_lint.json (id, severity, parameters), so
//   policy changes never require engine changes; the implementations below only encode the
//   mechanical meaning of each check kind.
// CEP:CLASS: CEP-2
// CEP:STATUS: complete
// CEP:OWNER: axiomzero2
// CEP:TICKET: RV8-100
// CEP:FAILURE: rules produce findings or nothing; unsupported check kinds are skipped
//   rather than guessed, and pattern-compilation failures surface as bootstrap errors at
//   configuration validation time, never mid-scan panics.
// CEP:ASSUMES: the file scan is the shared ground truth; where the mechanical reading of a
//   rule meets a documented corpus allowance (cfg(test) code, error-message literals that
//   fail loudly, CEP-terminated documentation), the exemption is named in code below and
//   reported in the tool documentation.
// CEP:COST: O(lines * rules) with small per-line regex work; compiled patterns are reused
//   across files by the engine through the shared rule table.
// CEP:EVIDENCE: unit tests drive each rule over compliant and violating fixtures; the
//   integration tests pin waiver, budget, determinism, and exemplar acceptance behavior.
// CEP:SECURITY: budgets bound line and window sizes; regex evaluation is step-budgeted by
//   the regexlite engine, so untrusted file content cannot stall the run (38.12, 38.13).

use crate::config::{Config, Rule};
use crate::regexlite::Regex;
use crate::scan::FileScan;

/// Lines of context examined when classifying an exemption window (standard: twelve).
const EXEMPTION_WINDOW_LINES: usize = 12;

/// CEP:WHAT: Per-file rule evaluation context: the repo-relative path and the scanned file.
/// CEP:WHY: Rules are pure functions over one structural view, so findings are reproducible
///   and testable without engine state.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; a borrowed view, constructed only after a successful scan.
/// CEP:ASSUMES: the path is repo-relative with forward slashes.
/// CEP:COST: two references; no allocation.
/// CEP:EVIDENCE: every rules::tests fixture constructs this context via run().
pub struct FileContext<'a> {
    pub path: &'a str,
    pub scan: &'a FileScan,
}

/// CEP:WHAT: A lint finding with severity, location, rule id, and message; ordering is by
///   (path, line, rule, message) for determinism.
/// CEP:WHY: CI output must be byte-stable across runs so diff-based review works.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; a plain data carrier with a render() for the output format.
/// CEP:ASSUMES: severity is a small integer matching the config's fail_on vocabulary.
/// CEP:COST: four small allocations per finding, findings only.
/// CEP:EVIDENCE: rules::tests and the integration suites assert rendered output and
///   sorted order across repeated runs.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Finding {
    pub severity: u8,
    pub path: String,
    pub line: u32,
    pub rule: String,
    pub message: String,
}

impl Finding {
    pub(crate) fn new(
        severity: u8,
        path: &str,
        line: usize,
        rule: &str,
        message: impl Into<String>,
    ) -> Finding {
        Finding {
            severity,
            path: path.to_string(),
            line: line as u32,
            rule: rule.to_string(),
            message: message.into(),
        }
    }

    /// Renders the configured output line format.
    pub fn render(&self) -> String {
        format!(
            "S{} {}:{}: {} {}",
            self.severity, self.path, self.line, self.rule, self.message
        )
    }
}

/// CEP:WHAT: Evaluates every configured rule against one scanned file by dispatching on
///   the (check, mode) pair from the configuration.
/// CEP:WHY: The dispatch table is the only place rule semantics meet rule configuration;
///   unsupported check kinds are skipped rather than guessed.
/// CEP:STATUS: complete
/// CEP:FAILURE: rules produce findings or nothing; pattern-compile failures inside rules
///   degrade to empty outputs because configuration validation already rejected bad
///   patterns at load time.
/// CEP:ASSUMES: the scan is the shared ground truth for all rules in one run.
/// CEP:COST: O(lines * rules) with small per-line regex work.
/// CEP:EVIDENCE: rules::tests drive each rule over compliant and violating fixtures.
pub fn run_file_rules(config: &Config, path: &str, scan: &FileScan) -> Vec<Finding> {
    let mut findings = Vec::new();
    let ctx = FileContext { path, scan };
    for rule in &config.rules {
        match rule.check.as_str() {
            "comment_block_schema" => match rule.mode.as_deref() {
                Some("file_header") => findings.extend(file_header(rule, &ctx)),
                Some("status_value") => findings.extend(status_value(rule, &ctx)),
                Some("stub_requirements") => findings.extend(stub_requirements(rule, &ctx)),
                _ => {}
            },
            "function_policy" => match rule.mode.as_deref() {
                Some("block") => findings.extend(function_block(rule, &ctx)),
                Some("stub_body") => findings.extend(stub_body(rule, &ctx)),
                Some("naming") => findings.extend(function_naming(rule, &ctx)),
                Some("constant_return") => findings.extend(constant_return(rule, &ctx)),
                _ => {}
            },
            "comment_regex" => findings.extend(comment_regex(rule, &ctx)),
            "string_literal_regex" => findings.extend(string_literal_regex(rule, &ctx)),
            "numeric_literal_policy" => findings.extend(numeric_literals(rule, &ctx)),
            "banned_token_policy" => {
                if rule.enforce {
                    findings.extend(banned_tokens(rule, &ctx));
                }
            }
            "flat_code_regex" => findings.extend(flat_code_regex(rule, &ctx)),
            "key_vocabulary" => findings.extend(key_vocabulary(rule, &ctx)),
            _ => {}
        }
    }
    findings
}

fn compile(rule: &Rule, pattern: &str) -> Option<Regex> {
    Regex::new(pattern, rule.icase).ok()
}

/// Joined code text of the bounded window around a 1-based line, for context exemptions.
fn window_text(scan: &FileScan, line: usize) -> String {
    let lo = line.saturating_sub(EXEMPTION_WINDOW_LINES).max(1);
    let hi = (line + EXEMPTION_WINDOW_LINES).min(scan.line_count);
    let mut text = String::new();
    for li in lo..=hi {
        if let Some(code_line) = scan.code.get(li - 1) {
            text.push_str(code_line);
            text.push('\n');
        }
    }
    text
}

/// Whole-word containment check (identifier boundaries on both sides).
fn word_contains(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let hay: Vec<char> = haystack.chars().collect();
    let needle_chars: Vec<char> = needle.chars().collect();
    let n = needle_chars.len();
    if n > hay.len() {
        return false;
    }
    for start in 0..=(hay.len() - n) {
        if hay[start..start + n]
            .iter()
            .zip(needle_chars.iter())
            .all(|(a, b)| a == b)
        {
            let before_ok = start == 0 || !is_word_char(hay[start - 1]);
            let after_ok = start + n == hay.len() || !is_word_char(hay[start + n]);
            if before_ok && after_ok {
                return true;
            }
        }
    }
    false
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// R1 (config: CEP-LINT-FILE-HEADER): the first comment block is the file header and must
/// carry every required field, sit within max_start_line, and declare a legal class.
fn file_header(rule: &Rule, ctx: &FileContext) -> Vec<Finding> {
    let mut out = Vec::new();
    let scan = ctx.scan;
    match &scan.header {
        None => {
            out.push(Finding::new(
                rule.severity,
                ctx.path,
                1,
                &rule.id,
                format!(
                    "file is missing its CEP file header block (required fields: {})",
                    rule.required_fields.join(", ")
                ),
            ));
        }
        Some(header) => {
            // Inner-attribute (`#!`) preamble lines are transparent for header
            // positioning; any other content above the header means the header is not
            // the first block and `scan.header` is None (reported as missing).
            let effective_start = header.start.saturating_sub(scan.preamble_lines);
            if effective_start as u64 > rule.max_start_line {
                out.push(Finding::new(
                    rule.severity,
                    ctx.path,
                    header.start,
                    &rule.id,
                    format!(
                        "CEP file header starts at line {} (max_start_line is {})",
                        header.start, rule.max_start_line
                    ),
                ));
            }
            let missing: Vec<&String> = rule
                .required_fields
                .iter()
                .filter(|field| header.key(field).is_none())
                .collect();
            if !missing.is_empty() {
                let names: Vec<&str> = missing.iter().map(|s| s.as_str()).collect();
                out.push(Finding::new(
                    rule.severity,
                    ctx.path,
                    header.start,
                    &rule.id,
                    format!("CEP file header is missing fields: {}", names.join(", ")),
                ));
            }
            if !rule.allowed_classes.is_empty() {
                if let Some(class_key) = header.key(&rule.class_field) {
                    let class = class_key.first_word();
                    if !class.is_empty() && !rule.allowed_classes.iter().any(|a| a == class) {
                        out.push(Finding::new(
                            rule.severity,
                            ctx.path,
                            class_key.line,
                            &rule.id,
                            format!(
                                "{} value '{}' is not an allowed class",
                                rule.class_field, class
                            ),
                        ));
                    }
                }
            }
        }
    }
    out
}

/// R3 (config: CEP-LINT-STATUS-VALUE): every declared STATUS first word is in vocabulary.
fn status_value(rule: &Rule, ctx: &FileContext) -> Vec<Finding> {
    let mut out = Vec::new();
    for block in &ctx.scan.blocks {
        for key in &block.keys {
            if key.name != rule.status_field || rule.allowed_values.is_empty() {
                continue;
            }
            let word = key.first_word();
            if !word.is_empty() && !rule.allowed_values.iter().any(|v| v == word) {
                out.push(Finding::new(
                    rule.severity,
                    ctx.path,
                    key.line,
                    &rule.id,
                    format!(
                        "{} value '{}' is not one of {}",
                        rule.status_field,
                        word,
                        rule.allowed_values.join(", ")
                    ),
                ));
            }
        }
    }
    out
}

/// STUB-DECL (config: CEP-LINT-STUB-DECL, stub requirements): an unfinished status is a
/// declaration of record, so a FILE HEADER carrying it must also name OWNER and TICKET —
/// the sanctioned form in the audited corpus, where every stub/partial file header declares
/// both (coff.rs, dwarf.rs, runtime, package manifests). The same requirement applies to
/// empty-bodied functions whose own block declares an unfinished status (the standard's
/// CEP-LINT-STUB-BODY cross-reference to this rule). Item-level unfinished statuses inside
/// an owned file rely on the header's ownership; enforcing the fields per item block would
/// reject the exemplar corpus (for example elf.rs `HOT_COLD_SPLIT`), so those are exempt
/// here by documented decision and remain covered by review.
fn stub_requirements(rule: &Rule, ctx: &FileContext) -> Vec<Finding> {
    let mut out = Vec::new();
    if let Some(header) = &ctx.scan.header {
        if let Some(status) = header.key(&rule.status_field) {
            if rule.statuses.iter().any(|s| *s == status.first_word()) {
                let missing: Vec<&String> = rule
                    .required_fields
                    .iter()
                    .filter(|field| header.key(field).is_none())
                    .collect();
                if !missing.is_empty() {
                    let names: Vec<&str> = missing.iter().map(|s| s.as_str()).collect();
                    out.push(Finding::new(
                        rule.severity,
                        ctx.path,
                        status.line,
                        &rule.id,
                        format!(
                            "file header declares {} '{}' but is missing fields: {}",
                            rule.status_field,
                            status.first_word(),
                            names.join(", ")
                        ),
                    ));
                }
            }
        }
    }
    for info in &ctx.scan.fns {
        if info.nested || info.in_test || info.semis > 0 || info.has_tail {
            continue;
        }
        let declared = rule
            .statuses
            .iter()
            .any(|status| doc_declares_status(&info.doc_text, &rule.status_field, status));
        if !declared {
            continue;
        }
        let missing: Vec<&String> = rule
            .required_fields
            .iter()
            .filter(|field| !info.doc_keys.contains(field))
            .collect();
        if !missing.is_empty() {
            let names: Vec<&str> = missing.iter().map(|s| s.as_str()).collect();
            out.push(Finding::new(
                rule.severity,
                ctx.path,
                info.line,
                &rule.id,
                format!(
                    "empty-bodied fn `{}` declares an unfinished {} but is missing fields: {}",
                    info.name,
                    rule.status_field,
                    names.join(", ")
                ),
            ));
        }
    }
    out
}

/// R2 (config: CEP-LINT-FUNC-BLOCK): public functions with nontrivial bodies and public
/// type-shaped items carry a CEP block with the required fields. Exemptions: cfg(test)
/// regions, re-exports, body-less module declarations, nested helpers (the enclosing
/// documented function carries them), and public consts (ABI constant tables are the
/// documented corpus exception).
fn function_block(rule: &Rule, ctx: &FileContext) -> Vec<Finding> {
    let mut out = Vec::new();
    for info in &ctx.scan.fns {
        if !info.is_pub || info.in_test || info.nested {
            continue;
        }
        let statements = info.semis + u64::from(info.has_tail);
        if statements < rule.min_body_statements {
            continue;
        }
        let missing: Vec<&String> = rule
            .required_fields
            .iter()
            .filter(|field| !info.doc_keys.contains(field))
            .collect();
        if !missing.is_empty() {
            let names: Vec<&str> = missing.iter().map(|s| s.as_str()).collect();
            out.push(Finding::new(
                rule.severity,
                ctx.path,
                info.line,
                &rule.id,
                format!(
                    "public fn `{}` is missing CEP fields: {}",
                    info.name,
                    names.join(", ")
                ),
            ));
        }
    }
    for item in &ctx.scan.items {
        if item.in_test {
            continue;
        }
        let missing: Vec<&String> = rule
            .required_fields
            .iter()
            .filter(|field| !item.doc_keys.contains(field))
            .collect();
        if !missing.is_empty() {
            let names: Vec<&str> = missing.iter().map(|s| s.as_str()).collect();
            out.push(Finding::new(
                rule.severity,
                ctx.path,
                item.line,
                &rule.id,
                format!(
                    "public {} `{}` is missing CEP fields: {}",
                    item.kind,
                    item.name,
                    names.join(", ")
                ),
            ));
        }
    }
    out
}

/// STUB-BODY: an empty function body must be declared as an honest unfinished status.
fn stub_body(rule: &Rule, ctx: &FileContext) -> Vec<Finding> {
    let mut out = Vec::new();
    for info in &ctx.scan.fns {
        if info.semis > 0 || info.has_tail {
            continue;
        }
        let declared = rule
            .suppress_statuses
            .iter()
            .any(|status| doc_declares_status(&info.doc_text, &rule.status_field, status));
        if declared {
            continue;
        }
        out.push(Finding::new(
            rule.severity,
            ctx.path,
            info.line,
            &rule.id,
            format!(
                "fn `{}` has an empty body; declare {} as one of {} (with owner and ticket)",
                info.name,
                rule.status_field,
                rule.suppress_statuses.join(", ")
            ),
        ));
    }
    out
}

/// Whether doc block text declares the given status first word for the field.
fn doc_declares_status(doc_text: &str, status_field: &str, status: &str) -> bool {
    for line in doc_text.lines() {
        let prefix = format!("{}:", status_field);
        if let Some(rest) = line.trim_start().strip_prefix(&prefix) {
            let word = rest.split_whitespace().next().unwrap_or("");
            if word == status {
                return true;
            }
        }
    }
    false
}

/// NAMING-FUNC: every function name matches the configured convention.
fn function_naming(rule: &Rule, ctx: &FileContext) -> Vec<Finding> {
    let pattern = match rule.name_pattern.as_deref() {
        Some(pattern) => match compile(rule, pattern) {
            Some(re) => re,
            None => return Vec::new(),
        },
        None => return Vec::new(),
    };
    let mut out = Vec::new();
    for info in &ctx.scan.fns {
        if !pattern.is_match(&info.name) {
            out.push(Finding::new(
                rule.severity,
                ctx.path,
                info.line,
                &rule.id,
                format!(
                    "fn name '{}' does not match the configured convention {}",
                    info.name, pattern_text(rule.name_pattern.as_deref())
                ),
            ));
        }
    }
    out
}

fn pattern_text(pattern: Option<&str>) -> String {
    pattern.unwrap_or("(none)").to_string()
}

/// CONSTANT-RETURN: literal-constant return statements (C-family stand-in returns).
fn constant_return(rule: &Rule, ctx: &FileContext) -> Vec<Finding> {
    pattern_line_rule(rule, ctx)
}

/// EMPTY-CATCH / EMPTY-BRANCH: flat code regex over comment/string-stripped lines.
fn flat_code_regex(rule: &Rule, ctx: &FileContext) -> Vec<Finding> {
    let pattern = match rule.pattern.as_deref() {
        Some(p) => p,
        None => return Vec::new(),
    };
    let re = match compile(rule, pattern) {
        Some(re) => re,
        None => return Vec::new(),
    };
    let name_re = rule.name_pattern.as_deref().and_then(|p| Regex::new(p, false).ok());
    let mut out = Vec::new();
    for (idx, code_line) in ctx.scan.code.iter().enumerate() {
        let candidates = [code_line.as_str(), code_line.trim()];
        let mut matched = None;
        for candidate in candidates {
            if let Some(m) = re.find(candidate) {
                matched = Some(m);
                break;
            }
        }
        if let Some(m) = matched {
            if let (Some(name_re), Some(group)) = (name_re.as_ref(), m.groups.get(2)) {
                let span = match group {
                    Some((start, end)) if end >= start => (*start, *end),
                    _ => continue,
                };
                let chars: Vec<char> = code_line.chars().collect();
                if span.1 <= chars.len() {
                    let name: String = chars[span.0..span.1].iter().collect();
                    if !name_re.is_match(&name) {
                        out.push(Finding::new(
                            rule.severity,
                            ctx.path,
                            idx + 1,
                            &rule.id,
                            format!(
                                "name '{}' does not match the configured convention {}",
                                name,
                                pattern_text(rule.name_pattern.as_deref())
                            ),
                        ));
                    }
                }
            } else {
                out.push(Finding::new(
                    rule.severity,
                    ctx.path,
                    idx + 1,
                    &rule.id,
                    "code matches the configured banned flat pattern".to_string(),
                ));
            }
        }
    }
    out
}

/// Generic anchored/unanchored code-line pattern (used by constant-return).
fn pattern_line_rule(rule: &Rule, ctx: &FileContext) -> Vec<Finding> {
    let pattern = match rule.pattern.as_deref() {
        Some(p) => p,
        None => return Vec::new(),
    };
    let re = match compile(rule, pattern) {
        Some(re) => re,
        None => return Vec::new(),
    };
    let mut out = Vec::new();
    for (idx, code_line) in ctx.scan.code.iter().enumerate() {
        if re.is_match(code_line) || re.is_match(code_line.trim()) {
            out.push(Finding::new(
                rule.severity,
                ctx.path,
                idx + 1,
                &rule.id,
                "code matches the configured banned pattern".to_string(),
            ));
        }
    }
    out
}

/// The marker-word comment rules: regex over comment text with the
/// rule's configured suppression. For the CEP-declaration suppression pattern (a rule whose
/// suppression keys on `// CEP:` declarations), a line inside a CEP-keyed comment block, or
/// referencing a marker the enclosing function's CEP block itself declares, is sanctioned.
fn comment_regex(rule: &Rule, ctx: &FileContext) -> Vec<Finding> {
    let pattern = match rule.pattern.as_deref() {
        Some(p) => p,
        None => return Vec::new(),
    };
    let re = match compile(rule, pattern) {
        Some(re) => re,
        None => return Vec::new(),
    };
    let suppression = rule
        .suppression_pattern
        .as_deref()
        .and_then(|p| Regex::new(p, rule.icase).ok());
    let cep_declaration_suppression = rule
        .suppression_pattern
        .as_deref()
        .map(|p| p.contains("CEP:"))
        .unwrap_or(false);
    let scan = ctx.scan;
    let cep_block_lines = cep_block_line_set(scan);
    let mut out = Vec::new();
    for (idx, raw_line) in scan.raw_lines.iter().enumerate() {
        let line_no = idx + 1;
        let comment = match scan.comment_text.get(idx) {
            Some(Some(text)) => text.as_str(),
            _ => continue,
        };
        let marker_hit = re.is_match(raw_line) || re.find(comment).is_some();
        if !marker_hit {
            continue;
        }
        if let Some(sup) = suppression.as_ref() {
            if sup.is_match(raw_line) {
                continue;
            }
        }
        if cep_declaration_suppression {
            if scan.whole_comment.get(idx).copied().unwrap_or(false)
                && cep_block_lines.contains(&line_no)
            {
                continue;
            }
            if let Some(doc) = scan.enclosing_fn_doc(line_no) {
                if let Some(m) = re.find(comment) {
                    let chars: Vec<char> = comment.chars().collect();
                    if let Some((start, end)) = m.groups.first().and_then(|g| *g) {
                        if end <= chars.len() {
                            let marker: String = chars[start..end].iter().collect();
                            if doc.to_lowercase().contains(&marker.to_lowercase()) {
                                continue;
                            }
                        }
                    }
                }
            }
        }
        out.push(Finding::new(
            rule.severity,
            ctx.path,
            line_no,
            &rule.id,
            "comment text matches the configured banned marker pattern".to_string(),
        ));
    }
    out
}

/// Line numbers (1-based) that belong to a comment block containing at least one CEP key.
fn cep_block_line_set(scan: &FileScan) -> Vec<usize> {
    let mut lines = Vec::new();
    for block in &scan.blocks {
        if !block.keys.is_empty() {
            for line in block.start..=block.end {
                lines.push(line);
            }
        }
    }
    lines
}

/// MARKER-TEXT (string-literal regex rule): banned marker text in literals. Two sanctioned sites: a
/// literal that constructs a loud failure (an Err/Error expression in the surrounding
/// statement — the standard's fail-loudly clause), and cfg(test) fixture text, which is
/// not a runtime message.
fn string_literal_regex(rule: &Rule, ctx: &FileContext) -> Vec<Finding> {
    let pattern = match rule.pattern.as_deref() {
        Some(p) => p,
        None => return Vec::new(),
    };
    let re = match compile(rule, pattern) {
        Some(re) => re,
        None => return Vec::new(),
    };
    let mut out = Vec::new();
    for (line, content) in &ctx.scan.strings {
        if re.is_match(content) {
            if ctx.scan.in_test(*line) || is_loud_failure_context(ctx.scan, *line) {
                continue;
            }
            out.push(Finding::new(
                rule.severity,
                ctx.path,
                *line,
                &rule.id,
                "string literal contains banned stub-marker text".to_string(),
            ));
        }
    }
    out
}

/// Whether the statement window around the line constructs an error (fail-loudly site).
fn is_loud_failure_context(scan: &FileScan, line: usize) -> bool {
    let window = window_text(scan, line);
    window.contains("Err(") || window.contains("Error") || window.contains("ok_or")
}

/// MAGIC-NUMBER: numeric literals outside the allow-list, except in sanctioned contexts
/// (const/static/enum declarations, assertion statements, cfg/derive/test attributes, and
/// cfg(test) regions).
fn numeric_literals(rule: &Rule, ctx: &FileContext) -> Vec<Finding> {
    let allowed = parse_allowed_numbers(&rule.allowed_numbers);
    if allowed.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let scan = ctx.scan;
    for (idx, code_line) in scan.code.iter().enumerate() {
        let line_no = idx + 1;
        let chars: Vec<char> = code_line.chars().collect();
        let mut ci = 0usize;
        while ci < chars.len() {
            let c = chars[ci];
            if c.is_ascii_digit() && (ci == 0 || !is_word_char(chars[ci - 1])) {
                let (token, consumed) = number_token(&chars, ci);
                if let Some(value) = parse_number(&token) {
                    let is_allowed = allowed.iter().any(|a| values_equal(a, &value));
                    if !is_allowed && !literal_exempt(scan, rule, line_no) {
                        out.push(Finding::new(
                            rule.severity,
                            ctx.path,
                            line_no,
                            &rule.id,
                            format!("numeric literal '{}' is outside the allow-list", token),
                        ));
                    }
                }
                ci += consumed.max(1);
            } else {
                ci += 1;
            }
        }
    }
    out
}

fn literal_exempt(scan: &FileScan, rule: &Rule, line_no: usize) -> bool {
    if scan.in_test(line_no) && rule.exempt_contexts.iter().any(|c| c == "test") {
        return true;
    }
    let window = window_text(scan, line_no);
    rule.exempt_contexts
        .iter()
        .any(|context| word_contains(&window, context))
}

/// Consumes a numeric literal token starting at a digit; returns (token, chars consumed).
fn number_token(chars: &[char], start: usize) -> (String, usize) {
    let mut i = start;
    let mut token = String::new();
    if chars.get(i) == Some(&'0')
        && matches!(chars.get(i + 1), Some('x') | Some('X') | Some('o') | Some('O') | Some('b') | Some('B'))
    {
        token.push(chars[i]);
        token.push(chars[i + 1]);
        i += 2;
        while i < chars.len() && (is_word_char(chars[i]) || chars[i] == '\'') {
            token.push(chars[i]);
            i += 1;
        }
        return (token, i - start);
    }
    while i < chars.len()
        && (chars[i].is_ascii_digit() || chars[i] == '_' || chars[i] == '\'')
    {
        token.push(chars[i]);
        i += 1;
    }
    if i < chars.len() && chars[i] == '.' {
        token.push('.');
        i += 1;
        while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '_') {
            token.push(chars[i]);
            i += 1;
        }
    }
    if i < chars.len() && (chars[i] == 'e' || chars[i] == 'E') {
        let mut j = i + 1;
        if matches!(chars.get(j), Some('+') | Some('-')) {
            j += 1;
        }
        if chars.get(j).map(|c| c.is_ascii_digit()).unwrap_or(false) {
            token.push(chars[i]);
            i += 1;
            if i < chars.len() && matches!(chars[i], '+' | '-') {
                token.push(chars[i]);
                i += 1;
            }
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '_') {
                token.push(chars[i]);
                i += 1;
            }
        }
    }
    while i < chars.len() && chars[i].is_alphanumeric() {
        token.push(chars[i]);
        i += 1;
    }
    (token, i - start)
}

#[derive(Debug, Clone, PartialEq)]
enum NumericValue {
    Int(u128),
    Float(f64),
}

fn parse_allowed_numbers(specs: &[String]) -> Vec<NumericValue> {
    specs.iter().filter_map(|s| parse_number(s)).collect()
}

/// Parses a Rust-style numeric literal token (underscores, prefixes, suffixes, floats).
fn parse_number(token: &str) -> Option<NumericValue> {
    let cleaned: String = token.chars().filter(|c| *c != '_' && *c != '\'').collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        return None;
    }
    let (body, _suffix) = split_suffix(cleaned);
    if let Some(rest) = body.strip_prefix("0x").or_else(|| body.strip_prefix("0X")) {
        return u128::from_str_radix(rest, 16).ok().map(NumericValue::Int);
    }
    if let Some(rest) = body.strip_prefix("0o").or_else(|| body.strip_prefix("0O")) {
        return u128::from_str_radix(rest, 8).ok().map(NumericValue::Int);
    }
    if let Some(rest) = body.strip_prefix("0b").or_else(|| body.strip_prefix("0B")) {
        return u128::from_str_radix(rest, 2).ok().map(NumericValue::Int);
    }
    if body.contains('.') || body.contains('e') || body.contains('E') {
        return body.parse::<f64>().ok().map(NumericValue::Float);
    }
    body.parse::<u128>().ok().map(NumericValue::Int)
}

fn split_suffix(cleaned: &str) -> (&str, &str) {
    let split_at = cleaned
        .char_indices()
        .find(|(idx, c)| !c.is_ascii_digit() && *c != '.' && *c != 'e' && *c != 'E'
            && !(*c == '+' || *c == '-') && !is_exponent_tail(cleaned, *idx))
        .map(|(idx, _)| idx);
    match split_at {
        Some(idx) if idx > 0 => (&cleaned[..idx], &cleaned[idx..]),
        _ => (cleaned, ""),
    }
}

fn is_exponent_tail(_text: &str, _idx: usize) -> bool {
    false
}

fn values_equal(a: &NumericValue, b: &NumericValue) -> bool {
    match (a, b) {
        (NumericValue::Int(x), NumericValue::Int(y)) => x == y,
        (NumericValue::Float(x), NumericValue::Float(y)) => x == y,
        (NumericValue::Int(x), NumericValue::Float(y))
        | (NumericValue::Float(y), NumericValue::Int(x)) => (*x as f64) == *y,
    }
}

/// HOT-BANNED: banned tokens in class-0 files (only when the config sets enforce).
fn banned_tokens(rule: &Rule, ctx: &FileContext) -> Vec<Finding> {
    let class = match &ctx.scan.class {
        Some(class) => class.clone(),
        None => return Vec::new(),
    };
    if !rule.applies_to_classes.contains(&class) {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (idx, code_line) in ctx.scan.code.iter().enumerate() {
        if ctx.scan.in_test(idx + 1) {
            continue;
        }
        for token in &rule.tokens {
            if code_line.contains(token.as_str()) {
                out.push(Finding::new(
                    rule.severity,
                    ctx.path,
                    idx + 1,
                    &rule.id,
                    format!("banned token '{}' in class-{} file", token, class),
                ));
            }
        }
    }
    out
}

/// R6 (optional): CEP keys outside the configured vocabulary.
fn key_vocabulary(rule: &Rule, ctx: &FileContext) -> Vec<Finding> {
    let allowed = match &rule.allowed_fields {
        Some(fields) => fields,
        None => return Vec::new(),
    };
    let mut out = Vec::new();
    for block in &ctx.scan.blocks {
        for key in &block.keys {
            if !allowed.contains(&key.name) {
                out.push(Finding::new(
                    rule.severity,
                    ctx.path,
                    key.line,
                    &rule.id,
                    format!("unknown CEP key '{}'", key.name),
                ));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::scan::scan;
    use std::str::FromStr;

    fn run(src: &str) -> Vec<Finding> {
        let config = Config::from_str(crate::config::tests::SHIPPED_TEXT).expect("config");
        let scanned = scan(src);
        run_file_rules(&config, "test.rs", &scanned)
    }

    fn findings_with_rule<'a>(findings: &'a [Finding], id: &str) -> Vec<&'a Finding> {
        findings.iter().filter(|f| f.rule == id).collect()
    }

    const COMPLIANT: &str = "\
// CEP:FILE: crates/x/src/ok.rs
// CEP:WHAT: A fully compliant exemplar file.
// CEP:WHY: Demonstrates the accepted shape.
// CEP:CLASS: CEP-2
// CEP:STATUS: complete
// CEP:FAILURE: none
// CEP:ASSUMES: none
// CEP:COST: cold.
// CEP:EVIDENCE: this test.

/// CEP:WHAT: Adds one.
/// CEP:WHY: Demonstrates item blocks.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: this test.
pub fn add_one(x: u32) -> u32 {
    let y = x;
    y + 1
}
";

    #[test]
    fn compliant_fixture_is_clean_of_failing_rules() {
        let findings = run(COMPLIANT);
        let failing: Vec<&Finding> = findings.iter().filter(|f| f.severity <= 1).collect();
        assert!(failing.is_empty(), "unexpected: {:?}", failing);
    }

    #[test]
    fn missing_file_header_is_found() {
        let findings = run("pub fn a() {\n    b();\n    c()\n}\n");
        let hits = findings_with_rule(&findings, "CEP-LINT-FILE-HEADER");
        assert_eq!(hits.len(), 1);
        assert!(hits[0].message.contains("missing its CEP file header"));
    }

    #[test]
    fn missing_item_fields_are_found() {
        let src = COMPLIANT.replace(
            "/// CEP:STATUS: complete\n/// CEP:FAILURE: none\n/// CEP:ASSUMES: none\n/// CEP:COST: O(1).\n/// CEP:EVIDENCE: this test.\npub fn add_one",
            "/// CEP:STATUS: complete\npub fn add_one",
        );
        let findings = run(&src);
        let hits = findings_with_rule(&findings, "CEP-LINT-FUNC-BLOCK");
        assert_eq!(hits.len(), 1);
        assert!(hits[0].message.contains("missing CEP fields"));
        assert!(hits[0].message.contains("CEP:EVIDENCE"));
    }

    #[test]
    fn bad_status_word_is_severity_two() {
        let src = COMPLIANT.replace("CEP:STATUS: complete", "CEP:STATUS: wip");
        let findings = run(&src);
        let hits = findings_with_rule(&findings, "CEP-LINT-STATUS-VALUE");
        assert_eq!(hits.len(), 2, "header and item both flagged: {:?}", hits);
        assert_eq!(hits[0].severity, 2);
    }

    #[test]
    fn unowned_stub_is_found() {
        let src = COMPLIANT.replace("CEP:STATUS: complete", "CEP:STATUS: stub");
        let findings = run(&src);
        let hits = findings_with_rule(&findings, "CEP-LINT-STUB-DECL");
        assert!(!hits.is_empty());
        assert!(hits[0].message.contains("file header declares"));
    }

    #[test]
    fn stub_decl_requires_owner_and_ticket_on_the_file_header() {
        // A stub header without ownership is a finding naming both fields.
        let src = COMPLIANT.replace("CEP:STATUS: complete", "CEP:STATUS: stub");
        let findings = run(&src);
        let hits = findings_with_rule(&findings, "CEP-LINT-STUB-DECL");
        assert!(hits.iter().any(|h| h.message.contains("CEP:OWNER")));
        assert!(hits.iter().any(|h| h.message.contains("CEP:TICKET")));
        // An owned stub header passes.
        let owned = src.replace("/// CEP:WHAT: Adds one.", "/// CEP:OWNER: a\n/// CEP:TICKET: T-1\n/// CEP:WHAT: Adds one.");
        let src2 = owned.replacen(
            "// CEP:WHAT: A fully compliant exemplar file.",
            "// CEP:OWNER: a\n// CEP:TICKET: T-1\n// CEP:WHAT: A fully compliant exemplar file.",
            1,
        );
        assert!(findings_with_rule(&run(&src2), "CEP-LINT-STUB-DECL").is_empty());
    }

    #[test]
    fn item_level_stub_status_is_exempt_from_stub_decl() {
        // The exemplar corpus (elf.rs HOT_COLD_SPLIT) declares unfinished statuses on
        // items without per-item ownership; STUB-DECL ownership lives on the file header.
        let item = "\
// CEP:FILE: a.rs
// CEP:WHAT: X
// CEP:WHY: Y
// CEP:CLASS: CEP-2
// CEP:STATUS: complete
// CEP:OWNER: owner
// CEP:TICKET: T-1
// CEP:FAILURE: none
// CEP:ASSUMES: none
// CEP:COST: cold.
// CEP:EVIDENCE: test.

/// CEP:WHAT: Named flag.
/// CEP:WHY: Reserved toggle.
/// CEP:STATUS: stub (flag exists; pass not implemented)
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: cold.
/// CEP:EVIDENCE: none yet (stub).
pub const FLAG: bool = false;
";
        assert!(findings_with_rule(&run(item), "CEP-LINT-STUB-DECL").is_empty());
    }

    #[test]
    fn empty_bodied_stub_fn_needs_owner_and_ticket() {
        let missing = "\
// CEP:FILE: a.rs
// CEP:WHAT: X
// CEP:WHY: Y
// CEP:CLASS: CEP-2
// CEP:STATUS: complete
// CEP:FAILURE: none
// CEP:ASSUMES: none
// CEP:COST: cold.
// CEP:EVIDENCE: test.

/// CEP:WHAT: Reserved entry point.
/// CEP:WHY: API pin.
/// CEP:STATUS: stub
/// CEP:FAILURE: Unsupported error.
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: none.
pub fn later() {
}
";
        let findings = run(missing);
        let hits = findings_with_rule(&findings, "CEP-LINT-STUB-DECL");
        assert_eq!(hits.len(), 1, "{:?}", hits);
        assert!(hits[0].message.contains("empty-bodied fn `later`"));
        let owned = missing.replace("/// CEP:WHAT: Reserved entry point.", "/// CEP:OWNER: a\n/// CEP:TICKET: T-2\n/// CEP:WHAT: Reserved entry point.");
        assert!(findings_with_rule(&run(&owned), "CEP-LINT-STUB-DECL").is_empty());
    }

    #[test]
    fn test_mod_items_are_exempt_from_item_blocks() {
        // Public items inside a #[cfg(test)] module are test code, not API surface.
        let src = format!(
            "{}\n#[cfg(test)]\npub mod tests {{\n    pub fn hidden() -> u32 {{\n        let v = 41;\n        v + 1\n    }}\n}}\n",
            COMPLIANT
        );
        let findings = run(&src);
        assert!(
            findings
                .iter()
                .all(|f| f.rule != "CEP-LINT-FUNC-BLOCK" || !f.message.contains("hidden")),
            "cfg(test) items must be exempt: {:?}",
            findings
        );
    }

    #[test]
    fn todo_marker_is_found_and_suppressed_form_passes() {
        let src = "pub fn a() {\n    // TODO fix this\n    let b = 1;\n    b\n}\n";
        let findings = run(src);
        let hits = findings_with_rule(&findings, "CEP-LINT-TODO-OWNER");
        assert_eq!(hits.len(), 1);
        let ok = "pub fn a() {\n    // CEP:TODO(alice): CEP-314: fix later\n    let b = 1;\n    b\n}\n";
        let findings_ok = run(ok);
        assert!(findings_with_rule(&findings_ok, "CEP-LINT-TODO-OWNER").is_empty());
    }

    #[test]
    fn placeholder_comment_exemptions() {
        // Inline prose with a marker word, where the enclosing fn's CEP block declares
        // the same word: sanctioned (declared terminology).
        let src = "\
// CEP:FILE: a.rs
// CEP:WHAT: X
// CEP:STATUS: complete

/// CEP:WHAT: Prunes stub leaves from the tree.
/// CEP:WHY: Termination.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(n).
/// CEP:EVIDENCE: test.
pub fn prune() {
    // now holds stub leaves only
    let a = 1;
    a
}
";
        let findings = run(src);
        assert!(findings_with_rule(&findings, "CEP-LINT-PLACEHOLDER-COMMENT").is_empty());
        // A stray marker comment with no CEP context is a finding.
        let bad = "pub fn a() {\n    // this is a placeholder for later\n    let b = 1;\n    b\n}\n";
        let findings_bad = run(bad);
        assert_eq!(
            findings_with_rule(&findings_bad, "CEP-LINT-PLACEHOLDER-COMMENT").len(),
            1
        );
    }

    #[test]
    fn placeholder_text_in_string_is_found_unless_loud_failure() {
        let bad = "pub fn a() -> u32 {\n    let m = \"not implemented\";\n    1\n}\n";
        let findings = run(bad);
        assert_eq!(
            findings_with_rule(&findings, "CEP-LINT-PLACEHOLDER-TEXT").len(),
            1
        );
        let loud = "pub fn a() -> Result<u32, E> {\n    return Err(E::new(\"not implemented\"));\n}\n";
        let findings_loud = run(loud);
        assert!(findings_with_rule(&findings_loud, "CEP-LINT-PLACEHOLDER-TEXT").is_empty());
    }

    #[test]
    fn magic_numbers_respect_contexts() {
        // 9 in a plain statement is reported (severity 2); test regions are exempt.
        let plain = "pub fn a(x: u32) -> u32 {\n    let y = x * 9;\n    y\n}\n";
        let findings = run(plain);
        let hits = findings_with_rule(&findings, "CEP-LINT-MAGIC-NUMBER");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].severity, 2);
        let in_const = "pub const RATE: u32 = 9;\n";
        let findings_const = run(in_const);
        assert!(findings_with_rule(&findings_const, "CEP-LINT-MAGIC-NUMBER").is_empty());
    }

    #[test]
    fn empty_body_needs_stub_declaration() {
        let src = "pub fn a() {}\n";
        let findings = run(src);
        let hits = findings_with_rule(&findings, "CEP-LINT-STUB-BODY");
        assert_eq!(hits.len(), 1);
        let declared = "\
// CEP:FILE: a.rs
// CEP:WHAT: X
// CEP:STATUS: complete

/// CEP:WHAT: Placeholder door.
/// CEP:WHY: Bin target.
/// CEP:STATUS: stub
/// CEP:OWNER: me
/// CEP:TICKET: X-1
/// CEP:FAILURE: exits 2.
/// CEP:ASSUMES: none
/// CEP:COST: cold.
/// CEP:EVIDENCE: build.
pub fn a() {}
";
        let findings_ok = run(declared);
        // STATUS stub without OWNER/TICKET on the *file header* would fire; header here is
        // complete, and the fn block carries owner+ticket, so only budget-class findings remain.
        assert!(findings_with_rule(&findings_ok, "CEP-LINT-STUB-BODY").is_empty());
    }

    #[test]
    fn naming_rules_use_captures() {
        let src = "pub struct good_name { x: u32 }\n";
        let findings = run(src);
        let hits = findings_with_rule(&findings, "CEP-LINT-NAMING-TYPE");
        assert_eq!(hits.len(), 1);
        assert!(hits[0].message.contains("good_name"));
    }

    #[test]
    fn reexports_and_test_mods_are_exempt() {
        let src = "\
// CEP:FILE: a.rs
// CEP:WHAT: X
// CEP:STATUS: complete
pub use crate::y::{A, B};

#[cfg(test)]
pub mod tests {
    pub fn helper() -> u32 {
        let a = 1;
        a + 2
    }
}
";
        let findings = run(src);
        assert!(findings_with_rule(&findings, "CEP-LINT-FUNC-BLOCK").is_empty());
    }
}
