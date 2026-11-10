// CEP:FILE: tools/cep_lint/src/checks_text.rs
// CEP:WHAT: The line-text rule family: flat code regexes, marker-word comment rules,
//   string-literal markers, the numeric literal policy, banned hot-path tokens, and the
//   small text helpers they share (exemption windows, word-boundary containment).
// CEP:WHY: These checks share one mechanical shape - a configured pattern evaluated over
//   one line class (code, comment, or literal) with documented exemptions - so they live
//   apart from the block-schema rules to keep both modules reviewable in one sitting
//   (chapter 62's atomic-file mandate).
// CEP:CLASS: CEP-2
// CEP:STATUS: complete
// CEP:OWNER: axiomzero2
// CEP:TICKET: RV8-147
// CEP:FAILURE: rules produce findings or nothing; pattern-compile failures degrade to no
//   findings because configuration validation already rejected bad patterns at load time.
// CEP:ASSUMES: the file scan is the shared ground truth; where the mechanical reading of a
//   rule meets a documented corpus allowance (cfg(test) code, error-message literals that
//   fail loudly, CEP-terminated documentation), the exemption is named in code below.
// CEP:COST: O(lines * rules) with small per-line regex work.
// CEP:EVIDENCE: checks_text::tests drive the marker, literal, numeric, and naming rules
//   over compliant and violating fixtures; rules::tests cover the block-schema family.
// CEP:SECURITY: budgets bound line and window sizes; regex evaluation is step-budgeted by
//   the regexlite engine, so untrusted file content cannot stall the run (38.12, 38.13).

use crate::config::Rule;
use crate::regexlite::Regex;
use crate::rules::{compile, is_word_char, pattern_text, Finding, FileContext};
use crate::scan::FileScan;

/// Lines of context examined when classifying an exemption window (standard: twelve).
const EXEMPTION_WINDOW_LINES: usize = 12;

/// CEP:WHAT: Joined code text of the bounded window around a 1-based line, for context
///   exemptions.
/// CEP:WHY: Context exemptions (fail-loudly literals, sanctioned keywords) need a bounded
///   neighborhood of stripped code, not the whole file.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; out-of-range lines clamp into the file's bounds.
/// CEP:ASSUMES: the scan's code lines are comment- and string-stripped.
/// CEP:COST: O(window) with window = 2 * EXEMPTION_WINDOW_LINES + 1 lines.
/// CEP:EVIDENCE: exercised by every string-literal and numeric exemption test.
pub(crate) fn window_text(scan: &FileScan, line: usize) -> String {
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

/// CEP:WHAT: Whole-word containment check (identifier boundaries on both sides).
/// CEP:WHY: Context exemptions match sanctioned keywords, and a substring hit inside a
///   longer identifier ("constantly" containing "const") must not count.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; an empty needle is always false.
/// CEP:ASSUMES: word characters are alphanumeric plus underscore.
/// CEP:COST: O(haystack * needle) worst case, both tiny in practice.
/// CEP:EVIDENCE: exercised by the numeric exemption tests via literal_exempt.
pub(crate) fn word_contains(haystack: &str, needle: &str) -> bool {
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

/// CEP:WHAT: EMPTY-CATCH / EMPTY-BRANCH and NAMING-TYPE: flat code regex over
///   comment/string-stripped lines, with the optional capture-and-name-pattern mode.
/// CEP:WHY: Empty control bodies and non-conforming type names are single-line shapes;
///   one code-line regex covers both mechanically.
/// CEP:STATUS: complete
/// CEP:FAILURE: findings sit at the offending line; pattern-compile failures degrade to
///   no findings (rejected at config validation).
/// CEP:ASSUMES: the scan's code lines are comment- and string-stripped; the name mode
///   uses capture group 2 of the configured pattern.
/// CEP:COST: O(lines) regex evaluations per configured rule.
/// CEP:EVIDENCE: rules::tests naming fixture and the empty-branch integration fixture.
pub(crate) fn flat_code_regex(rule: &Rule, ctx: &FileContext) -> Vec<Finding> {
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

/// CEP:WHAT: Generic anchored/unanchored code-line pattern rule (used by constant-return).
/// CEP:WHY: CONSTANT-RETURN bans literal-constant return statements, a single-line shape
///   the block-schema checks cannot express.
/// CEP:STATUS: complete
/// CEP:FAILURE: findings sit at the offending line; pattern-compile failures degrade to
///   no findings (rejected at config validation).
/// CEP:ASSUMES: the scan's code lines are comment- and string-stripped; both the raw and
///   trimmed line forms are tested so an anchored pattern may target either.
/// CEP:COST: O(lines) regex evaluations per configured rule.
/// CEP:EVIDENCE: exercised through the shipped CONSTANT-RETURN rule in the manifest.
pub(crate) fn pattern_line_rule(rule: &Rule, ctx: &FileContext) -> Vec<Finding> {
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

/// CEP:WHAT: The marker-word comment rules: regex over comment text with the rule's
///   configured suppression; CEP-declaration suppressions also honor CEP-keyed blocks
///   and enclosing-function CEP docs that declare the same marker.
/// CEP:WHY: TODO/FIXME/placeholder markers rot silently unless mechanically flagged, and
///   declared terminology in a CEP block is the standard's sanctioned form.
/// CEP:STATUS: complete
/// CEP:FAILURE: findings sit at the offending comment line; pattern-compile failures
///   degrade to no findings (rejected at config validation).
/// CEP:ASSUMES: comment text per line comes from the scan; a marker inside a CEP-keyed
///   whole-line comment block or inside the enclosing function's CEP doc is sanctioned.
/// CEP:COST: O(lines) regex evaluations plus O(blocks) for the CEP-line set.
/// CEP:EVIDENCE: checks_text::tests cover the marker hit, the suppression form, and the
///   declared-terminology exemption.
pub(crate) fn comment_regex(rule: &Rule, ctx: &FileContext) -> Vec<Finding> {
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

/// CEP:WHAT: MARKER-TEXT (string-literal regex rule): banned marker text in literals.
/// CEP:WHY: Stub wording hidden in string literals ships to users; the standard bans it
///   outside the two sanctioned sites.
/// CEP:STATUS: complete
/// CEP:FAILURE: findings sit at the literal's starting line; pattern-compile failures
///   degrade to no findings (rejected at config validation).
/// CEP:ASSUMES: two sanctioned sites are exempt: a literal constructing a loud failure
///   (Err/Error in the surrounding statement - the fail-loudly clause) and cfg(test)
///   fixture text, which is not a runtime message.
/// CEP:COST: O(literals) regex evaluations per configured rule.
/// CEP:EVIDENCE: checks_text::tests cover the plain hit, the loud-failure exemption, and
///   the cfg(test) exemption via the shipped rule.
pub(crate) fn string_literal_regex(rule: &Rule, ctx: &FileContext) -> Vec<Finding> {
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

/// CEP:WHAT: MAGIC-NUMBER: numeric literals outside the allow-list except in sanctioned
///   contexts (const/static/enum declarations, assertions, cfg/derive/test attributes,
///   cfg(test) regions).
/// CEP:WHY: Unnamed literals hide physical assumptions; the standard requires names or
///   sanctioned contexts.
/// CEP:STATUS: complete
/// CEP:FAILURE: findings sit at the offending literal; an empty allow-list disables the
///   rule; pattern-compile failures do not apply (no patterns here).
/// CEP:ASSUMES: tokens are Rust-style numerics (prefixes, underscores, suffixes, floats)
///   extracted from stripped code lines; word-boundary starts exclude identifier spans.
/// CEP:COST: O(total digits) per file with exact numeric comparison.
/// CEP:EVIDENCE: checks_text::tests cover the plain hit, the const exemption, and the
///   cfg(test) exemption via the shipped rule.
pub(crate) fn numeric_literals(rule: &Rule, ctx: &FileContext) -> Vec<Finding> {
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

/// CEP:WHAT: HOT-BANNED: banned tokens in class-0 files, evaluated only when the config
///   sets enforce.
/// CEP:WHY: Class-0 (hot) code has a hard allocation/panic/dynamic-dispatch ban; the
///   advisory-until-enforced switch lets the corpus adopt it in one deterministic flip.
/// CEP:STATUS: complete
/// CEP:FAILURE: findings sit at the offending token; a file without a declared class is
///   outside the check; pattern-compile failures do not apply (plain token match).
/// CEP:ASSUMES: applies_to_classes selects the bounded classes; cfg(test) regions are
///   exempt because test scaffolding is not shipped hot code.
/// CEP:COST: O(lines * tokens) plain containment checks, no regex work.
/// CEP:EVIDENCE: exercised through the shipped HOT-BANNED rule in the manifest.
pub(crate) fn banned_tokens(rule: &Rule, ctx: &FileContext) -> Vec<Finding> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::rules::run_file_rules;
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
    fn naming_rules_use_captures() {
        let src = "pub struct good_name { x: u32 }\n";
        let findings = run(src);
        let hits = findings_with_rule(&findings, "CEP-LINT-NAMING-TYPE");
        assert_eq!(hits.len(), 1);
        assert!(hits[0].message.contains("good_name"));
    }
}
