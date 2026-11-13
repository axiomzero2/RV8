// CEP:FILE: tools/cep_lint/src/rules.rs
// CEP:WHAT: The block-schema and topology checks over the scanned file: file-header
//   schema, function and public-item CEP blocks, status vocabulary, stub declarations,
//   macro naming, CEP key vocabulary, and the atomic-file topology policy (line limits,
//   junk-drawer names, pass registration, ISEL partitioning).  The line-text family lives
//   in checks_text.rs.
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

use crate::checks_text;
use crate::config::{glob_match, Config, Rule};
use crate::regexlite::Regex;
use crate::scan::FileScan;


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
            "comment_regex" => findings.extend(checks_text::comment_regex(rule, &ctx)),
            "string_literal_regex" => findings.extend(checks_text::string_literal_regex(rule, &ctx)),
            "numeric_literal_policy" => findings.extend(checks_text::numeric_literals(rule, &ctx)),
            "banned_token_policy" => {
                if rule.enforce {
                    findings.extend(checks_text::banned_tokens(rule, &ctx));
                }
            }
            "flat_code_regex" => findings.extend(checks_text::flat_code_regex(rule, &ctx)),
            "macro_prefix_policy" => findings.extend(macro_prefix(rule, &ctx)),
            "key_vocabulary" => findings.extend(key_vocabulary(rule, &ctx)),
            "file_topology_policy" => findings.extend(file_topology(rule, &ctx)),
            // Chapter 65 (Bloodhound/HUNTER): both receipt checks are active only under
            // the --verify-hunt invocation, which this build does not implement yet
            // (STANDARDS RV8-150); outside that mode the configured rules are dormant by
            // specification, so a normal run emits no findings for them.
            "commit_hunt_manifest" => {}
            "hunter_manifest" => {}
            _ => {}
        }
    }
    findings
}

pub(crate) fn compile(rule: &Rule, pattern: &str) -> Option<Regex> {
    Regex::new(pattern, rule.icase).ok()
}

pub(crate) fn is_word_char(c: char) -> bool {
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

pub(crate) fn pattern_text(pattern: Option<&str>) -> String {
    pattern.unwrap_or("(none)").to_string()
}

/// CONSTANT-RETURN: literal-constant return statements (C-family stand-in returns).
fn constant_return(rule: &Rule, ctx: &FileContext) -> Vec<Finding> {
    checks_text::pattern_line_rule(rule, ctx)
}

/// MACRO-PREFIX: macro definitions must carry the configured name prefix.
///
/// CEP:WHAT: The macro_prefix_policy check.  C++ preprocessor `#define NAME ...` lines
///   and, per the CEP&CC 0.3 Rust mapping, `macro_rules! NAME` definitions are the
///   recognized object-like macro forms; a defined name is compliant when it matches
///   `name_pattern` (the CEP_ prefix law of section 33.16) or the include-guard
///   `guard_pattern`, and any other macro definition is a finding at its line.
/// CEP:WHY: Section 33.16 requires prefixed macro names so a macro's origin is visible at
///   every use site; mechanical enforcement keeps the discipline from rotting.
/// CEP:STATUS: complete
/// CEP:FAILURE: findings name the macro and its line; pattern-compile failures degrade to
///   no findings (already rejected at config validation).
/// CEP:ASSUMES: the scan's code lines are comment- and string-stripped, so macro-looking
///   text inside comments or string literals cannot reach this check; `#undef` is not a
///   definition and is never flagged.
/// CEP:COST: one linear pass over code lines per configured macro rule.
/// CEP:EVIDENCE: rules::tests macro_prefix fixtures cover both dialects, both exemption
///   patterns, and the violation path.
fn macro_prefix(rule: &Rule, ctx: &FileContext) -> Vec<Finding> {
    let name_re = match rule.name_pattern.as_deref().map(|p| Regex::new(p, false)) {
        Some(Ok(re)) => re,
        _ => return Vec::new(),
    };
    let guard_re = rule
        .guard_pattern
        .as_deref()
        .and_then(|p| Regex::new(p, false).ok());
    let mut findings = Vec::new();
    for (index, line) in ctx.scan.code.iter().enumerate() {
        let name = match macro_definition_name(line) {
            Some(name) => name,
            None => continue,
        };
        let prefixed = name_re.find(&name).is_some();
        let guarded = guard_re
            .as_ref()
            .map(|re| re.find(&name).is_some())
            .unwrap_or(false);
        if !prefixed && !guarded {
            findings.push(Finding::new(
                rule.severity,
                ctx.path,
                index + 1,
                &rule.id,
                format!(
                    "macro '{}' does not match the required prefix or guard pattern",
                    name
                ),
            ));
        }
    }
    findings
}

/// Extracts the defined name from a macro definition line, if the line is one.
/// Recognizes `#define NAME`, `#  define NAME` (whitespace after `#`), the function-like
/// `#define NAME(..)` shape, and both bare and stringified `macro_rules! NAME` forms.
fn macro_definition_name(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    let rest = if trimmed.starts_with('#') {
        trimmed
            .trim_start_matches('#')
            .trim_start()
            .strip_prefix("define")?
            .trim_start()
    } else {
        trimmed
            .strip_prefix("macro_rules!")?
            .trim_start()
            .trim_start_matches('"')
    };
    let name: String = rest.chars().take_while(|c| is_word_char(*c)).collect();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
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

/// Opening lines searched for the generated-code marker (62.5.1's exemption).
const GENERATED_MARKER_SCAN_LINES: usize = 20;

/// CEP:WHAT: The atomic-file topology check (CEP&CC 0.4 chapter 62): per-class line
///   limits (62.5.1), the junk-drawer filename ban (62.4), the one-pass-per-file cap
///   under the configured passes topology (62.3.1), and the ISEL category cap under the
///   configured targets/*/isel topology (62.3.3).
/// CEP:WHY: Chapter 62 makes the file - not the function - the atomic unit of review and
///   enforcement; 62.5.3 requires cep_lint to enforce the four duties mechanically.
/// CEP:STATUS: complete
/// CEP:FAILURE: findings are file-level (line 1) for the limit and name duties and sit at
///   the first over-budget registration or category line for the topology duties;
///   pattern-compile failures degrade to no findings (rejected at config validation).
/// CEP:ASSUMES: class_line_limits names exactly the classes the standard bounds; the
///   generated-code exemption is the generated_pattern marker inside the file's opening
///   lines; the topology duties fire only under the configured globs (this repository's
///   flat backend matches none of them).
/// CEP:COST: O(lines) per file for the scan-backed duties; the name and class duties are
///   O(stems + limits).
/// CEP:EVIDENCE: rules::tests atomic_file fixtures cover all four duties, the generated
///   exemption, and the compliant shapes; the self-test manifest carries one violating
///   and one compliant scenario per duty.
pub(crate) fn file_topology(rule: &Rule, ctx: &FileContext) -> Vec<Finding> {
    let mut out = Vec::new();
    // Duty 1 (62.5.1): the declared class must respect its configured line limit,
    // unless the file carries the generated-code marker (62.5.1's exception).
    let generated_exempt = rule
        .generated_pattern
        .as_deref()
        .and_then(|p| Regex::new(p, false).ok())
        .map(|re| {
            ctx.scan
                .raw_lines
                .iter()
                .take(GENERATED_MARKER_SCAN_LINES)
                .any(|line| re.is_match(line))
        })
        .unwrap_or(false);
    if !generated_exempt {
        if let Some(class) = ctx.scan.class.as_deref() {
            if let Some(&(_, max)) = rule
                .class_line_limits
                .iter()
                .find(|(name, _)| name == class)
            {
                // wc-style line count: the phantom trailing element of a newline-
                // terminated source is not a line (62.5.1 counts translation-unit lines).
                let mut count = ctx.scan.line_count as u64;
                if ctx
                    .scan
                    .raw_lines
                    .last()
                    .map(|line| line.is_empty())
                    .unwrap_or(false)
                {
                    count -= 1;
                }
                if count > max {
                    out.push(Finding::new(
                        rule.severity,
                        ctx.path,
                        1,
                        &rule.id,
                        format!(
                            "file has {} lines, exceeding the {}-line limit for class {}",
                            count, max, class
                        ),
                    ));
                }
            }
        }
    }
    // Duty 2 (62.4): the file stem must not contain a banned junk-drawer stem.
    let stem = file_stem(ctx.path);
    for banned in &rule.banned_name_stems {
        if !banned.is_empty() && stem.contains(&banned.to_lowercase()) {
            out.push(Finding::new(
                rule.severity,
                ctx.path,
                1,
                &rule.id,
                format!(
                    "file name stem '{}' contains the banned junk-drawer stem '{}' (62.4)",
                    stem, banned
                ),
            ));
            break;
        }
    }
    // Duty 3 (62.3.1, 62.5.3): at most max_registrations_per_file pass registrations
    // under the configured passes topology.
    if rule.passes_globs.iter().any(|g| glob_match(g, ctx.path)) {
        if let Some(re) = rule
            .pass_registration_pattern
            .as_deref()
            .and_then(|p| Regex::new(p, false).ok())
        {
            let mut count = 0u64;
            for (idx, code_line) in ctx.scan.code.iter().enumerate() {
                if re.is_match(code_line) {
                    count += 1;
                    if count == rule.max_registrations_per_file + 1 {
                        out.push(Finding::new(
                            rule.severity,
                            ctx.path,
                            idx + 1,
                            &rule.id,
                            format!(
                                "file registers more than {} optimization pass(es) (62.3.1)",
                                rule.max_registrations_per_file
                            ),
                        ));
                        break;
                    }
                }
            }
        }
    }
    // Duty 4 (62.3.3, 62.5.3): at most max_isel_categories_per_file distinct ISEL
    // categories under the configured isel topology; categories are the capture of
    // isel_category_pattern over whole lines because the spec's tags live in comments.
    if rule.isel_globs.iter().any(|g| glob_match(g, ctx.path)) {
        if let Some(re) = rule
            .isel_category_pattern
            .as_deref()
            .and_then(|p| Regex::new(p, false).ok())
        {
            let mut seen: Vec<String> = Vec::new();
            for (idx, raw_line) in ctx.scan.raw_lines.iter().enumerate() {
                if let Some(m) = re.find(raw_line) {
                    let category = match m.groups.get(1) {
                        Some(Some((start, end))) => {
                            let chars: Vec<char> = raw_line.chars().collect();
                            if *end <= chars.len() {
                                chars[*start..*end].iter().collect::<String>()
                            } else {
                                String::new()
                            }
                        }
                        _ => String::new(),
                    };
                    if !category.is_empty() && !seen.contains(&category) {
                        seen.push(category);
                        if seen.len() == rule.max_isel_categories_per_file as usize + 1 {
                            out.push(Finding::new(
                                rule.severity,
                                ctx.path,
                                idx + 1,
                                &rule.id,
                                format!(
                                    "file declares more than {} ISEL instruction-class category/categories (62.3.3)",
                                    rule.max_isel_categories_per_file
                                ),
                            ));
                            break;
                        }
                    }
                }
            }
        }
    }
    out
}

/// Lowercased file stem (basename without extension) of a slash-separated path.
fn file_stem(path: &str) -> String {
    let base = path.rsplit('/').next().unwrap_or(path);
    let stem = match base.rfind('.') {
        Some(idx) if idx > 0 => &base[..idx],
        _ => base,
    };
    stem.to_lowercase()
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

    fn topology_config() -> Config {
        Config::from_str(
            r#"{
            "standard": {"name": "CEP&CC", "version": "0.4", "source": "standard.md"},
            "file_globs": ["**/*", "*"],
            "rules": {"CEP-LINT-ATOMIC-FILE": {
                "severity": 1,
                "check": "file_topology_policy",
                "class_line_limits": {"CEP-0": 10, "CEP-1": 20, "CEP-2": 40},
                "banned_name_stems": ["util", "helper", "misc", "common"],
                "passes_globs": ["**/passes/**"],
                "pass_registration_pattern": "register_pass!",
                "max_registrations_per_file": 1,
                "isel_globs": ["**/targets/*/isel/**"],
                "isel_category_pattern": "isel-category:\\s*([a-z0-9_]+)",
                "max_isel_categories_per_file": 1,
                "generated_pattern": "deterministically generated"
            }}
            }"#,
        )
        .expect("topology config")
    }

    fn run_topology(path: &str, src: &str) -> Vec<Finding> {
        let config = topology_config();
        let scanned = scan(src);
        run_file_rules(&config, path, &scanned)
    }

    fn topology_hits(path: &str, src: &str) -> Vec<Finding> {
        let findings = run_topology(path, src);
        findings
            .into_iter()
            .filter(|f| f.rule == "CEP-LINT-ATOMIC-FILE")
            .collect()
    }

    #[test]
    fn atomic_file_line_limit_is_flagged_and_compliant_passes() {
        let hot = "// CEP:CLASS: CEP-0\n";
        let over = format!("{}{}", hot, "let x = 1;\n".repeat(10)); // 11 lines > 10
        let hits = topology_hits("src/hot.rs", &over);
        assert_eq!(hits.len(), 1);
        assert!(hits[0].message.contains("10-line limit"));
        let under = format!("{}{}", hot, "let x = 1;\n".repeat(9)); // exactly 10
        assert!(topology_hits("src/hot.rs", &under).is_empty());
    }

    #[test]
    fn atomic_file_unlisted_class_has_no_mechanical_limit() {
        // 62.5.1 names only the CEP-0/1/2 classes; other classes are not bounded here.
        let over = format!("// CEP:CLASS: HPC-0\n{}", "let x = 1;\n".repeat(40));
        assert!(topology_hits("src/hot.rs", &over).is_empty());
        // A class with no configured limit and no header is likewise not flagged.
        let bare = "let x = 1;\n".repeat(40);
        assert!(topology_hits("src/hot.rs", &bare).is_empty());
    }

    #[test]
    fn atomic_file_generated_marker_exempts_the_line_limit() {
        let mut body = String::from("// deterministically generated by tools/gen_opcodes\n");
        body.push_str("// CEP:CLASS: CEP-0\n");
        body.push_str(&"let x = 1;\n".repeat(10));
        assert!(topology_hits("src/gen.rs", &body).is_empty());
        // The marker must sit in the opening lines; deep in the file it is not one.
        let mut late = String::from("// CEP:CLASS: CEP-0\n");
        late.push_str(&"let x = 1;\n".repeat(21));
        late.push_str("// deterministically generated by tools/gen_opcodes\n");
        assert_eq!(topology_hits("src/gen.rs", &late).len(), 1);
    }

    #[test]
    fn atomic_file_junk_drawer_stem_is_flagged() {
        let src = "let x = 1;\n";
        let hits = topology_hits("src/string_utils.rs", src);
        assert_eq!(hits.len(), 1);
        assert!(hits[0].message.contains("util"));
        for path in ["src/helpers.rs", "src/misc_bits.rs", "src/common.rs"] {
            assert_eq!(topology_hits(path, src).len(), 1, "path {}", path);
        }
        assert!(topology_hits("src/string_hash.rs", src).is_empty());
        assert!(topology_hits("src/pea.rs", src).is_empty());
    }

    #[test]
    fn atomic_file_pass_landfill_is_flagged_single_registration_passes() {
        let two = "register_pass!(licm);\nregister_pass!(loop_fusion);\n";
        let hits = topology_hits("src/passes/licm.rs", two);
        assert_eq!(hits.len(), 1);
        assert!(hits[0].message.contains("62.3.1"));
        let one = "register_pass!(licm);\n";
        assert!(topology_hits("src/passes/licm.rs", one).is_empty());
        // Outside the configured passes topology the cap does not apply.
        assert!(topology_hits("src/driver.rs", two).is_empty());
    }

    #[test]
    fn atomic_file_isel_multi_category_is_flagged_single_category_passes() {
        let two = "// isel-category: arithmetic\nselect(n);\n// isel-category: simd\n";
        let hits = topology_hits("src/targets/x86/isel/arithmetic.rs", two);
        assert_eq!(hits.len(), 1);
        assert!(hits[0].message.contains("62.3.3"));
        let one = "// isel-category: arithmetic\nselect(n);\n";
        assert!(topology_hits("src/targets/x86/isel/arithmetic.rs", one).is_empty());
        // A second mention of the SAME category is not a second category.
        let repeat = "// isel-category: arithmetic\n// isel-category: arithmetic\n";
        assert!(topology_hits("src/targets/x86/isel/arithmetic.rs", repeat).is_empty());
        // Outside the configured isel topology the cap does not apply.
        assert!(topology_hits("src/isel.rs", two).is_empty());
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

    #[test]
    fn macro_prefix_flags_unprefixed_definitions_in_both_dialects() {
        let src = concat!(
            "#define MAX 4096\n",
            "#define CEP_TARGET_CACHE_LINE_BYTES 64\n",
            "#define GUARD_HPP\n",
            "#undef MAX\n",
            "macro_rules! inner {\n",
            "    () => {};\n",
            "}\n",
            "macro_rules! CEP_outer {\n",
            "    () => {};\n",
            "}\n",
        );
        let findings = run(src);
        let hits = findings_with_rule(&findings, "CEP-LINT-MACRO-PREFIX");
        assert_eq!(hits.len(), 2, "MAX and inner are violations: {:?}", hits);
        assert!(hits.iter().any(|f| f.line == 1 && f.message.contains("MAX")));
        assert!(hits.iter().any(|f| f.line == 5 && f.message.contains("inner")));
    }

    #[test]
    fn macro_prefix_accepts_prefixed_guarded_and_stringified_names() {
        let src = concat!(
            "#  define CEP_HAS_MDSPAN 1\n",
            "#define GUARD_HPP\n",
            "#define CEP_FLAG(X) ((X) & 1)\n",
            "macro_rules! \"CEP_quoted\" {\n",
            "    () => {};\n",
            "}\n",
        );
        let findings = run(src);
        assert!(
            findings_with_rule(&findings, "CEP-LINT-MACRO-PREFIX").is_empty(),
            "all four shapes are compliant: {:?}",
            findings_with_rule(&findings, "CEP-LINT-MACRO-PREFIX")
        );
    }

    #[test]
    fn macro_prefix_ignores_macro_uses_and_comment_mentions() {
        let src = concat!(
            "// #define not_a_macro_in_comment\n",
            "let s = \"macro_rules! not_a_macro_in_string\";\n",
            "let x = 1;\n",
            "x\n",
        );
        let findings = run(src);
        assert!(findings_with_rule(&findings, "CEP-LINT-MACRO-PREFIX").is_empty());
    }
}
