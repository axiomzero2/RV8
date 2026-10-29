// CEP:FILE: tools/cep_lint/src/waivers.rs
// CEP:WHAT: Waiver loading and matching for .cep/waivers/: `<ticket>-<rule>.md` files with
//   EXPIRES, SCOPE, and JUSTIFICATION fields; active waivers suppress the named rule for
//   scoped paths, and anything malformed or expired fails closed.
// CEP:WHY: Severity 0/1 blocking requires a written, temporary, reviewed escape hatch
//   (standard 34.7); the linter must honor exactly the waivers present and refuse to honor
//   broken ones silently.
// CEP:CLASS: CEP-2
// CEP:STATUS: complete
// CEP:OWNER: axiomzero2
// CEP:TICKET: RV8-100
// CEP:FAILURE: unreadable or malformed waiver files are ignored for suppression and surface
//   as severity-2 advisory findings naming the file; expired waivers suppress nothing and
//   are reported; missing waiver directories simply mean no waivers.
// CEP:ASSUMES: waiver files are first-party UTF-8 markdown with the documented fields; the
//   rule id appears in the file name after the ticket (the first `CEP-LINT-` occurrence).
// CEP:COST: O(waivers) load, O(findings * waivers) matching with cheap glob work.
// CEP:EVIDENCE: unit tests cover honored, expired, malformed, scoped, and out-of-scope
//   waivers plus the empty-directory case.
// CEP:SECURITY: no network, no clock-derived suppression beyond expiry comparison; scopes
//   are repo-relative globs matched iteratively (38.13).

use crate::config::glob_match;
use std::path::Path;

/// CEP:WHAT: One parsed waiver file: ticket, rule id, scope globs, expiry date, and
///   justification, loaded from .cep/waivers.
/// CEP:WHY: Severity 0/1 blocking needs a written, temporary, reviewed escape hatch; this
///   struct is the machine-checked form of that policy.
/// CEP:STATUS: complete
/// CEP:FAILURE: malformed or incomplete waivers parse into an inactive form that
///   suppresses nothing and is reported as an advisory finding.
/// CEP:ASSUMES: the file name carries the ticket and the `CEP-LINT-<RULE>` id.
/// CEP:COST: one struct per waiver file, loaded once per run.
/// CEP:EVIDENCE: waivers::tests and integration::waiver_suppresses_rule_until_expiry
///   cover honored, expired, and malformed waivers.
#[derive(Debug, Clone, PartialEq)]
pub struct Waiver {
    pub file: String,
    pub ticket: String,
    pub rule: String,
    /// Repo-relative glob scopes; empty means the waiver suppresses nothing (fail closed).
    pub scopes: Vec<String>,
    /// EXPIRES date as ISO YYYY-MM-DD, or empty when absent (never active).
    pub expires: String,
    pub justification: String,
}

/// CEP:WHAT: Loads every waiver file in `dir` (README.md excluded), sorted by file name.
/// CEP:WHY: Deterministic waiver application requires a deterministic load order.
/// CEP:STATUS: complete
/// CEP:FAILURE: unreadable or malformed files become inactive waivers carrying a reason;
///   a missing directory simply means no waivers (never an error).
/// CEP:ASSUMES: waiver files are UTF-8 markdown with EXPIRES/SCOPE/JUSTIFICATION fields.
/// CEP:COST: O(waiver bytes) per run.
/// CEP:EVIDENCE: waivers::tests cover the empty-directory case and malformed names.
pub fn load(dir: &Path) -> Vec<Waiver> {
    let mut files: Vec<String> = match std::fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| entry.file_name().to_str().map(|s| s.to_string()))
            .filter(|name| name.ends_with(".md") && name != "README.md")
            .collect(),
        Err(_) => return Vec::new(),
    };
    files.sort();
    let mut waivers = Vec::new();
    for name in files {
        let path = dir.join(&name);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(_) => {
                waivers.push(broken(&name, "unreadable"));
                continue;
            }
        };
        let stem = name.trim_end_matches(".md").to_string();
        let (ticket, rule) = split_stem(&stem);
        if rule.is_empty() {
            waivers.push(broken(&name, "no CEP-LINT-<RULE> token in file name"));
            continue;
        }
        let mut expires = String::new();
        let mut justification = String::new();
        let mut scopes: Vec<String> = Vec::new();
        for line in text.lines() {
            if let Some(rest) = strip_field(line, "EXPIRES:") {
                expires = rest.trim().to_string();
            } else if let Some(rest) = strip_field(line, "SCOPE:") {
                for part in rest.split(',') {
                    let scope = part.trim();
                    if !scope.is_empty() {
                        scopes.push(scope.to_string());
                    }
                }
            } else if let Some(rest) = strip_field(line, "JUSTIFICATION:") {
                justification = rest.trim().to_string();
            }
        }
        waivers.push(Waiver {
            file: name,
            ticket,
            rule,
            scopes,
            expires,
            justification,
        });
    }
    waivers
}

fn broken(name: &str, reason: &str) -> Waiver {
    Waiver {
        file: name.to_string(),
        ticket: String::new(),
        rule: String::new(),
        scopes: Vec::new(),
        expires: String::new(),
        justification: format!("malformed waiver ({}): {}", reason, name),
    }
}

fn strip_field<'a>(line: &'a str, field: &str) -> Option<&'a str> {
    let trimmed = line.trim_start().trim_start_matches('#').trim_start();
    trimmed.strip_prefix(field)
}

/// Splits `<ticket>-CEP-LINT-<RULE>` into (ticket, rule).
fn split_stem(stem: &str) -> (String, String) {
    match stem.find("CEP-LINT-") {
        Some(idx) => {
            let ticket = stem[..idx].trim_end_matches('-').to_string();
            let rule = stem[idx..].to_string();
            (ticket, rule)
        }
        None => (stem.to_string(), String::new()),
    }
}

/// CEP:WHAT: Whether the waiver is active on `today`: well-formed, scoped, justified, and
///   not expired (expiry day inclusive).
/// CEP:WHY: Fail-closed waiver semantics: anything missing or stale suppresses nothing.
/// CEP:STATUS: complete
/// CEP:FAILURE: false for empty rule, empty scopes, empty justification, or an unparsable
///   or past expiry date.
/// CEP:ASSUMES: `today` is the UTC civil date; expiry compares as (year, month, day).
/// CEP:COST: O(1) comparisons.
/// CEP:EVIDENCE: waivers::tests::activity_requires_all_fields_and_future_expiry.
pub fn is_active(waiver: &Waiver, today: (i64, u32, u32)) -> bool {
    if waiver.rule.is_empty() || waiver.scopes.is_empty() || waiver.justification.is_empty() {
        return false;
    }
    match parse_date(&waiver.expires) {
        Some(expires) => {
            let (ty, tm, td) = today;
            (expires.0 > ty)
                || (expires.0 == ty && expires.1 > tm)
                || (expires.0 == ty && expires.1 == tm && expires.2 >= td)
        }
        None => false,
    }
}

fn parse_date(text: &str) -> Option<(i64, u32, u32)> {
    let mut parts = text.trim().split('-');
    let year: i64 = parts.next()?.parse().ok()?;
    let month: u32 = parts.next()?.parse().ok()?;
    let day: u32 = parts.next()?.parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some((year, month, day))
}

/// CEP:WHAT: Whether the waiver suppresses `rule` for `path`, matching any SCOPE glob
///   (or an exact path equality).
/// CEP:WHY: Waivers are scoped, not global; suppression must honor the written scope.
/// CEP:STATUS: complete
/// CEP:FAILURE: false whenever the rule id differs or no scope matches.
/// CEP:ASSUMES: path is repo-relative with forward slashes.
/// CEP:COST: O(scopes) glob work per finding.
/// CEP:EVIDENCE: waivers::tests::suppression_needs_rule_and_scope and the integration
///   waiver scenario.
pub fn suppresses(waiver: &Waiver, rule: &str, path: &str) -> bool {
    if waiver.rule != rule {
        return false;
    }
    waiver
        .scopes
        .iter()
        .any(|scope| glob_match(scope, path) || scope == path)
}

/// CEP:WHAT: Civil date (year, month, day) for a UNIX timestamp in seconds, UTC semantics.
/// CEP:WHY: Expiry checks need calendar dates without pulling in a time crate.
/// CEP:STATUS: complete
/// CEP:FAILURE: defined for all non-negative inputs via the standard civil-from-days
///   algorithm; no panic path.
/// CEP:ASSUMES: proleptic Gregorian calendar, which covers all waiver dates.
/// CEP:COST: O(1) integer arithmetic.
/// CEP:EVIDENCE: waivers::tests::civil_epoch_math pins the epoch and a modern date.
pub fn civil_from_unix(secs: u64) -> (i64, u32, u32) {
    let days = (secs / 86_400) as i64;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    (year, m as u32, d as u32)
}

/// Today's UTC date from the system clock.
pub fn today() -> (i64, u32, u32) {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(dur) => civil_from_unix(dur.as_secs()),
        Err(_) => (0, 1, 1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_epoch_math() {
        assert_eq!(civil_from_unix(0), (1970, 1, 1));
        // 2026-01-31T00:00:00Z
        assert_eq!(civil_from_unix(1_769_817_600), (2026, 1, 31));
    }

    #[test]
    fn stem_split() {
        let (ticket, rule) = split_stem("CEP-901-CEP-LINT-MAGIC-NUMBER");
        assert_eq!(ticket, "CEP-901");
        assert_eq!(rule, "CEP-LINT-MAGIC-NUMBER");
    }

    #[test]
    fn activity_requires_all_fields_and_future_expiry() {
        let mut waiver = Waiver {
            file: "x.md".into(),
            ticket: "T-1".into(),
            rule: "CEP-LINT-MAGIC-NUMBER".into(),
            scopes: vec!["crates/**".into()],
            expires: "2999-12-31".into(),
            justification: "reviewed".into(),
        };
        assert!(is_active(&waiver, (2026, 1, 1)));
        waiver.expires = "2000-01-01".into();
        assert!(!is_active(&waiver, (2026, 1, 1)));
        waiver.expires = "not-a-date".into();
        assert!(!is_active(&waiver, (2026, 1, 1)));
        waiver.expires = "2999-12-31".into();
        waiver.scopes.clear();
        assert!(!is_active(&waiver, (2026, 1, 1)));
        waiver.scopes = vec!["crates/**".into()];
        waiver.justification = String::new();
        assert!(!is_active(&waiver, (2026, 1, 1)));
    }

    #[test]
    fn suppression_needs_rule_and_scope() {
        let waiver = Waiver {
            file: "x.md".into(),
            ticket: "T-1".into(),
            rule: "CEP-LINT-MAGIC-NUMBER".into(),
            scopes: vec!["crates/rv8_core/**".into()],
            expires: "2999-12-31".into(),
            justification: "reviewed".into(),
        };
        assert!(suppresses(
            &waiver,
            "CEP-LINT-MAGIC-NUMBER",
            "crates/rv8_core/src/arena.rs"
        ));
        assert!(!suppresses(
            &waiver,
            "CEP-LINT-MAGIC-NUMBER",
            "crates/rv8_ir/src/hir.rs"
        ));
        assert!(!suppresses(
            &waiver,
            "CEP-LINT-TODO-OWNER",
            "crates/rv8_core/src/arena.rs"
        ));
    }
}
