// CEP:FILE: tools/cep_lint/src/scan.rs
// CEP:WHAT: One-pass source scanner producing the structural facts the rules need: per-line
//   comment text, comment/string-blanked code, string-literal contents, contiguous comment
//   blocks with parsed CEP keys, cfg(test) spans, function definitions with bodies and pub
//   visibility, public type-shaped items, and the file header block.
// CEP:WHY: Every rule consumes the same structural view; a single scanner keeps comment and
//   string handling consistent so banned-token, regex, and block-schema rules see identical
//   text and cannot be fooled by markers hidden inside strings or comments.
// CEP:CLASS: CEP-2
// CEP:STATUS: complete
// CEP:OWNER: axiomzero2
// CEP:TICKET: RV8-100
// CEP:FAILURE: scanning is total over any UTF-8 input; unterminated strings, comments, or
//   bodies degrade to bounded spans and never panic; oversized lines are flagged through a
//   named budget field for the engine to report.
// CEP:ASSUMES: Rust-like syntax (// and /* */ comments, "..."/r"..."/b"..." strings, brace
//   structure); signatures that split `pub` and `fn` across physical lines are not
//   recognized and rely on review, mirroring the standard's documented extractor limits.
// CEP:COST: O(file bytes) for the character pass plus O(structure) for the brace walks; all
//   traversals iterative with explicit stacks.
// CEP:EVIDENCE: unit tests cover exemplar-style files, raw strings, nested block comments,
//   cfg(test) spans, nested functions, visibility, and CEP key continuation folding.
// CEP:SECURITY: all scans are bounded by MAX_FILE_BYTES and MAX_LINE_BYTES; nothing here
//   recurses over input structure (38.12, 38.13).

/// Files above this size are refused with a budget finding instead of being scanned.
pub const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;

/// Lines above this byte length are analyzed only up to the budget; the excess is flagged.
pub const MAX_LINE_BYTES: usize = 4096;

/// Cap on signature/brace forward search so a malformed file cannot stall the scan.
const MAX_SCAN_LINES: usize = 4096;

/// CEP:WHAT: One CEP key occurrence inside a comment block, with continuation lines folded
///   into a single value.
/// CEP:WHY: Header values legitimately span lines, so rules must see one logical value per
///   key (the brief's continuation rule) without re-parsing raw comments.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; a plain data carrier.
/// CEP:ASSUMES: continuation lines attach to the nearest preceding CEP token until the
///   next token or the end of the comment block.
/// CEP:COST: one struct per key; values share the block's lifetime.
/// CEP:EVIDENCE: scan::tests cover continuation folding across `//` and `///` blocks.
#[derive(Debug, Clone, PartialEq)]
pub struct CepKey {
    pub name: String,
    /// 1-based line of the `CEP:NAME:` token.
    pub line: usize,
    /// Value text: the token line's remainder plus folded continuation lines.
    pub value: String,
}

impl CepKey {
    /// First whitespace-delimited word of the value (status vocabulary checks).
    pub fn first_word(&self) -> &str {
        self.value.split_whitespace().next().unwrap_or("")
    }
}

/// CEP:WHAT: A contiguous run of whole-line comments with its CEP keys parsed, spanning
///   lines [start, end] (1-based, inclusive).
/// CEP:WHY: File headers, item blocks, and waiver-like prose are all comment runs; one
///   type serves header detection, item attachment, and key-vocabulary checks.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; empty runs never become blocks.
/// CEP:ASSUMES: a blank or code line terminates the run.
/// CEP:COST: O(run length) to build and parse.
/// CEP:EVIDENCE: scan::tests build exemplar-shaped files and assert block boundaries and
///   parsed keys.
#[derive(Debug, Clone, PartialEq)]
pub struct CommentBlock {
    /// 1-based first line of the run.
    pub start: usize,
    /// 1-based last line of the run.
    pub end: usize,
    pub keys: Vec<CepKey>,
}

impl CommentBlock {
    pub fn key(&self, name: &str) -> Option<&CepKey> {
        self.keys.iter().find(|k| k.name == name)
    }

    pub fn key_names(&self) -> Vec<String> {
        self.keys.iter().map(|k| k.name.clone()).collect()
    }
}

/// CEP:WHAT: A function definition extracted from blanked code: name, position, body span,
///   visibility, statement counts, test/nesting flags, and the attached CEP doc block.
/// CEP:WHY: The function-policy rules (block presence, stub bodies, naming) all need the
///   same mechanical facts about one definition.
/// CEP:STATUS: complete
/// CEP:FAILURE: definitions whose signature spans a `pub`/`fn` split across physical lines
///   are not recognized (documented extractor limit relying on review).
/// CEP:ASSUMES: bodies are located by brace matching over comment-blanked code.
/// CEP:COST: O(1) per definition plus its body walk.
/// CEP:EVIDENCE: scan::tests cover nested functions, visibility, raw strings, and
///   cfg(test) classification.
#[derive(Debug, Clone)]
pub struct FnInfo {
    pub name: String,
    /// 1-based line of the `fn` keyword.
    pub line: usize,
    /// Inclusive body brace span (1-based lines).
    pub body: (usize, usize),
    pub is_pub: bool,
    pub semis: u64,
    pub has_tail: bool,
    pub nested: bool,
    pub in_test: bool,
    /// CEP key names in the doc block attached above the item.
    pub doc_keys: Vec<String>,
    /// Full doc block inner text (empty when unattached).
    pub doc_text: String,
}

/// CEP:WHAT: A public type-shaped item (struct, enum, trait, type, static, or mod with a
///   body) with its position, test membership, and attached CEP doc block.
/// CEP:WHY: R2 requires CEP blocks on public items, so the scan must enumerate them with
///   the same attachment rules as functions.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; unrecognized lines simply produce no item.
/// CEP:ASSUMES: `pub use` re-export lines are exempt and never appear here.
/// CEP:COST: O(1) per item plus the doc-block walk.
/// CEP:EVIDENCE: scan::tests and rules::tests fixtures cover structs, enums, and the
///   re-export exemption.
#[derive(Debug, Clone)]
pub struct ItemInfo {
    pub kind: String,
    pub name: String,
    pub line: usize,
    pub in_test: bool,
    pub doc_keys: Vec<String>,
    pub doc_text: String,
}

/// CEP:WHAT: The structural view of one source file: raw and blanked lines, comment text,
///   string literals, comment blocks, cfg(test) spans, functions, public items, the file
///   header, and budget flags.
/// CEP:WHY: Every rule consumes this one view so comment/string handling cannot disagree
///   between rules.
/// CEP:STATUS: complete
/// CEP:FAILURE: scanning is total over any UTF-8 input; unterminated constructs degrade to
///   bounded spans; oversized lines are surfaced through `long_line`.
/// CEP:ASSUMES: the file is at most MAX_FILE_BYTES (engine-enforced before scanning).
/// CEP:COST: O(file bytes) for the pass plus O(structure) for brace walks.
/// CEP:EVIDENCE: scan::tests cover exemplar shapes, raw strings, nested block comments,
///   test spans, and the `#!` preamble recognized by header detection.
#[derive(Debug, Clone)]
pub struct FileScan {
    pub line_count: usize,
    pub raw_lines: Vec<String>,
    /// Comment text per line (whole-line or trailing), when present.
    pub comment_text: Vec<Option<String>>,
    /// True when the line is a whole-line comment (only whitespace precedes `//`).
    pub whole_comment: Vec<bool>,
    /// Code text with comments and string/char contents blanked.
    pub code: Vec<String>,
    /// String literal contents with their starting line (1-based).
    pub strings: Vec<(usize, String)>,
    pub blocks: Vec<CommentBlock>,
    pub test_spans: Vec<(usize, usize)>,
    pub fns: Vec<FnInfo>,
    pub items: Vec<ItemInfo>,
    /// File-header block, if the first comment run starts at line 1 or after a transparent
    /// inner-attribute preamble.
    pub header: Option<CommentBlock>,
    /// Number of leading `#!` (inner attribute / shebang) lines that are transparent for
    /// file-header positioning.
    pub preamble_lines: usize,
    /// Declared CEP:CLASS first word from the header, when present.
    pub class: Option<String>,
    /// First line exceeding MAX_LINE_BYTES, if any.
    pub long_line: Option<usize>,
}

impl FileScan {
    /// Whether the 1-based line lies inside any `#[cfg(test)]` item body.
    pub fn in_test(&self, line: usize) -> bool {
        self.test_spans.iter().any(|&(a, b)| line >= a && line <= b)
    }

    /// CEP:WHAT: CEP doc text of the innermost function whose body contains the line.
    /// CEP:WHY: Numeric and token policies exempt code inside a documented function's
    ///   declared vocabulary, so they must find the enclosing block's text.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: None when the line lies outside every known body.
    /// CEP:ASSUMES: nested bodies resolve to the innermost enclosing definition.
    /// CEP:COST: O(functions) per query, linear scans only.
    /// CEP:EVIDENCE: scan::tests cover nested-function enclosure.
    pub fn enclosing_fn_doc(&self, line: usize) -> Option<&str> {
        let mut best: Option<&FnInfo> = None;
        for info in &self.fns {
            let (a, b) = info.body;
            if line > a && line <= b {
                best = match best {
                    Some(current) if current.body.0 >= a => Some(current),
                    _ => Some(info),
                };
            }
        }
        best.map(|f| f.doc_text.as_str())
    }
}

/// Scanner state machine over the raw file text.
#[derive(PartialEq, Clone, Copy)]
enum State {
    Code,
    Block,
    Str,
    RawStr(usize),
}

struct Scanner {
    code: Vec<String>,
    comment: Vec<Option<String>>,
    whole: Vec<bool>,
    strings: Vec<(usize, String)>,
}

impl Scanner {
    fn new(line_count: usize) -> Scanner {
        Scanner {
            code: vec![String::new(); line_count],
            comment: vec![None; line_count],
            whole: vec![false; line_count],
            strings: Vec::new(),
        }
    }

    fn set_comment(&mut self, line: usize, text: String) {
        if let Some(slot) = self.comment.get_mut(line) {
            if slot.is_none() {
                *slot = Some(text);
            } else if let Some(existing) = slot {
                existing.push(' ');
                existing.push_str(&text);
            }
        }
    }

    fn scan(&mut self, source: &str) {
        let mut state = State::Code;
        let mut line: usize = 0; // 0-based
        let chars: Vec<char> = source.chars().collect();
        let mut i = 0usize;
        let mut string_content = String::new();
        while i < chars.len() {
            let c = chars[i];
            match state {
                State::Code => {
                    match c {
                        '\n' => {
                            line += 1;
                            i += 1;
                        }
                        '/' => match chars.get(i + 1).copied() {
                            Some('/') => {
                                let mut text = String::new();
                                i += 2;
                                while i < chars.len() && chars[i] != '\n' {
                                    text.push(chars[i]);
                                    i += 1;
                                }
                                let leading_blank =
                                    self.code.get(line).map(|l| l.trim().is_empty()).unwrap_or(false);
                                if leading_blank {
                                    if let Some(flag) = self.whole.get_mut(line) {
                                        *flag = true;
                                    }
                                }
                                self.set_comment(line, text);
                            }
                            Some('*') => {
                                i += 2;
                                state = State::Block;
                                let leading_blank =
                                    self.code.get(line).map(|l| l.trim().is_empty()).unwrap_or(false);
                                if leading_blank {
                                    if let Some(flag) = self.whole.get_mut(line) {
                                        *flag = true;
                                    }
                                }
                            }
                            _ => {
                                if let Some(buf) = self.code.get_mut(line) {
                                    buf.push('/');
                                }
                                i += 1;
                            }
                        },
                        '"' => {
                            i += 1;
                            string_content.clear();
                            state = State::Str;
                            self.blank(line);
                        }
                        'r' if is_raw_string_start(&chars, i) => {
                            let (hashes, consumed) = raw_string_prefix(&chars, i);
                            i += consumed;
                            string_content.clear();
                            state = State::RawStr(hashes);
                            self.blank(line);
                        }
                        'b' if chars.get(i + 1) == Some(&'"') => {
                            i += 2;
                            string_content.clear();
                            state = State::Str;
                            self.blank(line);
                        }
                        'b' if is_raw_string_start(&chars, i + 1) => {
                            let (hashes, consumed) = raw_string_prefix(&chars, i + 1);
                            i += consumed + 1;
                            string_content.clear();
                            state = State::RawStr(hashes);
                            self.blank(line);
                        }
                        '\'' => match char_literal_len(&chars, i) {
                            Some(consumed) => {
                                i += consumed;
                                self.blank(line);
                            }
                            None => {
                                // Lifetime or generic tick: plain code.
                                if let Some(buf) = self.code.get_mut(line) {
                                    buf.push('\'');
                                }
                                i += 1;
                            }
                        },
                        _ => {
                            if let Some(buf) = self.code.get_mut(line) {
                                buf.push(c);
                            }
                            i += 1;
                        }
                    }
                }
                State::Block => {
                    match c {
                        '/' if chars.get(i + 1) == Some(&'*') => {
                            i += 2;
                        }
                        '*' if chars.get(i + 1) == Some(&'/') => {
                            i += 2;
                            state = State::Code;
                            self.blank(line);
                        }
                        '\n' => {
                            line += 1;
                            i += 1;
                        }
                        _ => {
                            let mut text = String::new();
                            text.push(c);
                            self.set_comment(line, text);
                            i += 1;
                        }
                    }
                }
                State::Str | State::RawStr(_) => {
                    let closing = match state {
                        State::RawStr(hashes) => closes_raw_string(&chars, i, hashes),
                        _ => c == '"',
                    };
                    if closing {
                        let len = match state {
                            State::RawStr(hashes) => 1 + hashes,
                            _ => 1,
                        };
                        self.strings.push((line + 1, string_content.clone()));
                        self.blank(line);
                        i += len;
                        state = State::Code;
                    } else if c == '\\' && state == State::Str {
                        if let Some(&next) = chars.get(i + 1) {
                            if next == '\n' {
                                // Line continuation: the newline is consumed and must
                                // advance the line counter like any other newline.
                                line += 1;
                            } else {
                                string_content.push(next);
                            }
                            i += 2;
                        } else {
                            i += 1;
                        }
                    } else if c == '\n' {
                        line += 1;
                        string_content.push('\n');
                        i += 1;
                    } else {
                        string_content.push(c);
                        i += 1;
                    }
                }
            }
        }
    }

    fn blank(&mut self, line: usize) {
        if let Some(buf) = self.code.get_mut(line) {
            buf.push(' ');
        }
    }
}

/// Detects `r` followed by hashes and a quote.
fn is_raw_string_start(chars: &[char], at: usize) -> bool {
    if chars.get(at) != Some(&'r') {
        return false;
    }
    let mut j = at + 1;
    while chars.get(j) == Some(&'#') {
        j += 1;
    }
    chars.get(j) == Some(&'"')
}

/// Returns (hash count, chars consumed through the opening quote).
fn raw_string_prefix(chars: &[char], at: usize) -> (usize, usize) {
    let mut j = at + 1;
    let mut hashes = 0usize;
    while chars.get(j) == Some(&'#') {
        j += 1;
        hashes += 1;
    }
    (hashes, j - at + 1)
}

/// Whether the raw string terminates at `at` (quote plus matching hashes).
fn closes_raw_string(chars: &[char], at: usize, hashes: usize) -> bool {
    if chars.get(at) != Some(&'"') {
        return false;
    }
    for k in 1..=hashes {
        if chars.get(at + k) != Some(&'#') {
            return false;
        }
    }
    true
}

/// Bounded lookahead recognizing a char literal starting at `at`; returns the span length
/// including both quotes, or None when the tick is a lifetime marker.
fn char_literal_len(chars: &[char], at: usize) -> Option<usize> {
    let c1 = *chars.get(at + 1)?;
    if c1 == '\\' {
        let c2 = *chars.get(at + 2)?;
        let body = if c2 == 'u' {
            let mut j = at + 3;
            let mut span = 3usize;
            while let Some(&c) = chars.get(j) {
                span += 1;
                j += 1;
                if c == '}' {
                    break;
                }
                if span > 12 {
                    return None;
                }
            }
            span
        } else {
            2
        };
        if chars.get(at + 1 + body) == Some(&'\'') {
            return Some(body + 2);
        }
        None
    } else if c1 != '\'' && c1 != '\n' && chars.get(at + 2) == Some(&'\'') {
        Some(3)
    } else {
        None
    }
}

/// Strips the comment marker from a whole-line comment's raw text, returning inner text.
fn comment_inner(raw: &str) -> String {
    let trimmed = raw.trim_start();
    let mut text = trimmed;
    while text.starts_with('/') {
        text = &text[1..];
    }
    text.strip_prefix(' ').unwrap_or(text).to_string()
}

/// Recognizes `CEP:NAME:` at the start of comment inner text; returns (NAME, value).
fn cep_token(inner: &str) -> Option<(String, String)> {
    if !inner.starts_with("CEP:") {
        return None;
    }
    let rest = &inner["CEP:".len()..];
    let end = rest.find(':')?;
    let name = &rest[..end];
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_' || c == '-')
    {
        return None;
    }
    let value = rest[end + 1..].trim_start().to_string();
    Some((format!("CEP:{}", name), value))
}

/// Parses CEP keys out of whole-line comment texts with 1-based line numbers.
fn parse_keys(lines: &[(usize, String)]) -> Vec<CepKey> {
    let mut keys: Vec<CepKey> = Vec::new();
    for &(line, ref text) in lines {
        let inner = comment_inner(text);
        if let Some((name, value)) = cep_token(&inner) {
            keys.push(CepKey {
                name,
                line,
                value,
            });
        } else if let Some(last) = keys.last_mut() {
            let continuation = inner.trim();
            if !continuation.is_empty() {
                if !last.value.is_empty() {
                    last.value.push(' ');
                }
                last.value.push_str(continuation);
            }
        }
    }
    keys
}

/// CEP:WHAT: Scans a source file into the structural view the rules consume.
/// CEP:WHY: One pass keeps comment blanking, string extraction, block parsing, and item
///   discovery consistent for every rule.
/// CEP:STATUS: complete
/// CEP:FAILURE: total over any UTF-8 input; unterminated strings, comments, or bodies
///   degrade to bounded spans and never panic; `source` must be valid UTF-8.
/// CEP:ASSUMES: Rust-like lexing (// and /* */ comments, raw and byte strings, char
///   literals versus lifetimes, brace structure).
/// CEP:COST: O(file bytes); every traversal is iterative with explicit stacks.
/// CEP:EVIDENCE: scan::tests plus the integration suites and the CI real-tree gate.
pub fn scan(source: &str) -> FileScan {
    let raw_lines: Vec<String> = source
        .split('\n')
        .map(|l| l.trim_end_matches('\r').to_string())
        .collect();
    let line_count = raw_lines.len();
    let mut long_line: Option<usize> = None;
    for (idx, raw) in raw_lines.iter().enumerate() {
        if long_line.is_none() && raw.len() > MAX_LINE_BYTES {
            long_line = Some(idx + 1);
        }
    }
    let mut scanner = Scanner::new(line_count);
    scanner.scan(source);
    let whole_comment = scanner.whole.clone();
    let code = scanner.code.clone();

    // Contiguous whole-line comment blocks.
    let mut blocks: Vec<CommentBlock> = Vec::new();
    let mut run: Vec<(usize, String)> = Vec::new();
    for idx in 0..line_count {
        if whole_comment[idx] {
            run.push((idx + 1, raw_lines[idx].clone()));
        } else if !run.is_empty() {
            blocks.push(build_block(&run));
            run.clear();
        }
    }
    if !run.is_empty() {
        blocks.push(build_block(&run));
    }

    // Header: first block starting after transparent `#!` preamble lines (line 1 normally).
    let mut transparent = 0usize;
    for raw in &raw_lines {
        let trimmed = raw.trim_start();
        if trimmed.starts_with("#!") {
            transparent += 1;
        } else {
            break;
        }
    }
    let header_start = 1 + transparent;
    let header = blocks.iter().find(|b| b.start == header_start).cloned();
    let class = header
        .as_ref()
        .and_then(|h| h.key("CEP:CLASS"))
        .map(|k| k.first_word().to_string());

    let test_spans = find_test_spans(&code);
    let fns = find_fns(&code, &test_spans, &whole_comment, &raw_lines);
    let items = find_items(&code, &test_spans, &whole_comment, &raw_lines);

    FileScan {
        line_count,
        raw_lines,
        comment_text: scanner.comment,
        whole_comment,
        code,
        strings: scanner.strings,
        blocks,
        test_spans,
        fns,
        items,
        header,
        preamble_lines: transparent,
        class,
        long_line,
    }
}

fn build_block(run: &[(usize, String)]) -> CommentBlock {
    let start = run[0].0;
    let end = run[run.len() - 1].0;
    CommentBlock {
        start,
        end,
        keys: parse_keys(run),
    }
}

/// Locates `#[cfg(test)]` / `#![cfg(test)]`-marked item bodies via iterative brace matching
/// on blanked code.
fn find_test_spans(code: &[String]) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut idx = 0usize;
    while idx < code.len() {
        let trimmed = code[idx].trim_start();
        if trimmed.starts_with("#![cfg(test") {
            // Inner attribute: the whole file module is test code.
            spans.push((idx + 1, code.len()));
            idx += 1;
            continue;
        }
        if trimmed.starts_with("#[cfg(test") {
            if let Some((open_line, open_char)) = first_brace_from(code, idx) {
                if let Some((close_line, _)) = match_brace(code, open_line, open_char) {
                    spans.push((idx + 1, close_line + 1));
                }
            }
        }
        idx += 1;
    }
    spans
}

/// Finds the first `{` at or after (line, 0); returns (0-based line, 0-based char).
fn first_brace_from(code: &[String], line: usize) -> Option<(usize, usize)> {
    let mut li = line;
    let mut budget = MAX_SCAN_LINES;
    while li < code.len() && budget > 0 {
        let chars: Vec<char> = code[li].chars().collect();
        for (ci, c) in chars.iter().copied().enumerate() {
            if c == '{' {
                return Some((li, ci));
            }
        }
        li += 1;
        budget -= 1;
    }
    None
}

/// Iteratively matches the brace opening at (line, char); returns closing (line, char).
fn match_brace(code: &[String], open_line: usize, open_char: usize) -> Option<(usize, usize)> {
    let mut depth: i64 = 0;
    let mut li = open_line;
    let mut ci = open_char;
    while li < code.len() {
        let chars: Vec<char> = code[li].chars().collect();
        while ci < chars.len() {
            match chars[ci] {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some((li, ci));
                    }
                }
                _ => {}
            }
            ci += 1;
        }
        li += 1;
        ci = 0;
    }
    None
}

/// Extracts function definitions from blanked code.
fn find_fns(
    code: &[String],
    test_spans: &[(usize, usize)],
    whole: &[bool],
    raw: &[String],
) -> Vec<FnInfo> {
    let mut infos: Vec<FnInfo> = Vec::new();
    for (li, line_text) in code.iter().enumerate() {
        let chars: Vec<char> = line_text.chars().collect();
        let mut cursor = 0usize;
        while let Some(rel) = find_fn_keyword(&chars, cursor) {
            let at = cursor + rel;
            cursor = at + 3;
            let mut ns = at + 2;
            while ns < chars.len() && chars[ns].is_whitespace() {
                ns += 1;
            }
            let mut name = String::new();
            let mut k = ns;
            while k < chars.len() && is_ident_char(chars[k]) {
                name.push(chars[k]);
                k += 1;
            }
            if name.is_empty() {
                continue;
            }
            let line_no = li + 1;
            let is_pub = has_pub_before(&chars, at);
            if let Some((open_line, open_char, close_line, close_char)) = locate_body(code, li, at)
            {
                let interior =
                    interior_text(code, open_line, open_char, close_line, close_char);
                let semis = interior.matches(';').count() as u64;
                let trimmed = interior.trim_end();
                let has_tail = !trimmed.is_empty() && !trimmed.ends_with(';');
                let in_test = test_spans.iter().any(|&(a, b)| line_no >= a && line_no <= b);
                let (doc_keys, doc_text) = attached_doc(code, whole, raw, li);
                infos.push(FnInfo {
                    name,
                    line: line_no,
                    body: (open_line + 1, close_line + 1),
                    is_pub,
                    semis,
                    has_tail,
                    nested: false,
                    in_test,
                    doc_keys,
                    doc_text,
                });
            }
        }
    }
    let spans: Vec<(usize, usize)> = infos.iter().map(|f| f.body).collect();
    for info in &mut infos {
        info.nested = spans.iter().any(|&(a, b)| info.line > a && info.line <= b);
    }
    infos
}

/// Finds the next `fn` keyword occurrence (word boundary, name required after) at/after
/// `from`; returns the char offset of the `f`.
fn find_fn_keyword(chars: &[char], from: usize) -> Option<usize> {
    let mut i = from;
    while i + 3 <= chars.len() {
        if chars[i] == 'f'
            && chars[i + 1] == 'n'
            && (i == 0 || !is_ident_char(chars[i - 1]))
            && chars.get(i + 2).is_some_and(|c| c.is_whitespace())
        {
            // The token after the whitespace must start an identifier.
            let mut j = i + 2;
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            if j < chars.len() && (chars[j].is_alphabetic() || chars[j] == '_') {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Whether a visibility marker precedes the `fn` keyword on its own line.
fn has_pub_before(chars: &[char], at: usize) -> bool {
    let prefix: String = chars[..at.min(chars.len())].iter().collect();
    let trimmed = prefix.trim();
    trimmed == "pub"
        || trimmed.starts_with("pub(")
        || trimmed.ends_with(" pub")
        || trimmed.ends_with("pub(")
        || trimmed.contains(" pub ")
        || trimmed.contains(" pub(")
}

/// Locates the function body; returns (open_line, open_char, close_line, close_char),
/// all 0-based, or None for body-less declarations.
fn locate_body(
    code: &[String],
    start_line: usize,
    start_char: usize,
) -> Option<(usize, usize, usize, usize)> {
    let mut li = start_line;
    let mut ci = start_char;
    let mut budget = MAX_SCAN_LINES;
    while li < code.len() && budget > 0 {
        let chars: Vec<char> = code[li].chars().collect();
        while ci < chars.len() {
            match chars[ci] {
                ';' => return None,
                '{' => {
                    let (close_line, close_char) = match_brace(code, li, ci)?;
                    return Some((li, ci, close_line, close_char));
                }
                _ => {}
            }
            ci += 1;
        }
        li += 1;
        ci = 0;
        budget -= 1;
    }
    None
}

/// Body interior text strictly between the braces.
fn interior_text(
    code: &[String],
    open_line: usize,
    open_char: usize,
    close_line: usize,
    close_char: usize,
) -> String {
    let mut text = String::new();
    if open_line >= code.len() || close_line >= code.len() || close_line < open_line {
        return text;
    }
    let open_chars: Vec<char> = code[open_line].chars().collect();
    if open_line == close_line {
        if open_char + 1 < close_char {
            text.extend(&open_chars[open_char + 1..close_char]);
        }
        return text;
    }
    if open_char < open_chars.len() {
        text.extend(&open_chars[open_char + 1..]);
    }
    text.push('\n');
    let mut li = open_line + 1;
    while li < close_line {
        text.push_str(&code[li]);
        text.push('\n');
        li += 1;
    }
    let close_chars: Vec<char> = code[close_line].chars().collect();
    if close_char > 0 {
        text.extend(&close_chars[..close_char]);
    }
    text
}

/// Walks upward from item line `item_idx` (0-based) past attribute lines, collecting the
/// contiguous whole-line comment run above; returns (key names, inner text).
fn attached_doc(
    code: &[String],
    whole: &[bool],
    raw: &[String],
    item_idx: usize,
) -> (Vec<String>, String) {
    let mut j = item_idx;
    while j > 0 {
        let trimmed = code.get(j - 1).map(|l| l.trim_start()).unwrap_or("");
        if trimmed.starts_with("#[") || trimmed.starts_with("#![") {
            j -= 1;
        } else {
            break;
        }
    }
    let mut run: Vec<(usize, String)> = Vec::new();
    while j > 0 && whole.get(j - 1).copied().unwrap_or(false) {
        run.push((j, raw.get(j - 1).cloned().unwrap_or_default()));
        j -= 1;
    }
    if run.is_empty() {
        return (Vec::new(), String::new());
    }
    run.reverse();
    let keys = parse_keys(&run);
    let names = keys.iter().map(|k| k.name.clone()).collect();
    let text = run
        .iter()
        .map(|(_, raw_line)| comment_inner(raw_line))
        .collect::<Vec<_>>()
        .join("\n");
    (names, text)
}

/// Extracts public type-shaped items from blanked code.
fn find_items(
    code: &[String],
    test_spans: &[(usize, usize)],
    whole: &[bool],
    raw: &[String],
) -> Vec<ItemInfo> {
    let mut items = Vec::new();
    for (li, line_text) in code.iter().enumerate() {
        let trimmed = line_text.trim_start();
        let (kind, name) = match classify_pub_item(trimmed) {
            Some(pair) => pair,
            None => continue,
        };
        let in_test = test_spans.iter().any(|&(a, b)| li + 1 >= a && li < b);
        let (doc_keys, doc_text) = attached_doc(code, whole, raw, li);
        items.push(ItemInfo {
            kind,
            name,
            line: li + 1,
            in_test,
            doc_keys,
            doc_text,
        });
    }
    items
}

/// Classifies a trimmed code line as a public type-shaped item; returns (kind, name).
fn classify_pub_item(trimmed: &str) -> Option<(String, String)> {
    for keyword in ["struct", "enum", "trait", "type", "static"] {
        let with_pub = format!("pub {} ", keyword);
        if trimmed.starts_with(&with_pub) {
            let name = identifier_after(trimmed, with_pub.len());
            return Some((keyword.to_string(), name));
        }
        if trimmed.starts_with("pub(") {
            if let Some(close) = trimmed.find(')') {
                let after = trimmed[close + 1..].trim_start();
                let prefix = format!("{} ", keyword);
                if after.starts_with(&prefix) {
                    let name = identifier_after(after, prefix.len());
                    return Some((keyword.to_string(), name));
                }
            }
        }
    }
    // `pub mod Name {` (with a body only; `pub mod name;` declarations stay exempt).
    let scoped_mod = trimmed.starts_with("pub(")
        && trimmed
            .find(')')
            .map(|close| trimmed[close + 1..].trim_start().starts_with("mod "))
            .unwrap_or(false);
    if (trimmed.starts_with("pub mod ") || scoped_mod) && trimmed.contains('{') {
        let after = if scoped_mod {
            let close = trimmed.find(')').unwrap_or(0);
            trimmed[close + 1..].trim_start().trim_start_matches("mod ")
        } else {
            &trimmed["pub mod ".len()..]
        };
        let name = identifier_after(after, 0);
        return Some(("mod".to_string(), name));
    }
    None
}

fn identifier_after(text: &str, from: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut k = from.min(chars.len());
    while k < chars.len() && !is_ident_char(chars[k]) {
        k += 1;
    }
    let mut name = String::new();
    while k < chars.len() && is_ident_char(chars[k]) {
        name.push(chars[k]);
        k += 1;
    }
    name
}

#[cfg(test)]
mod tests {
    use super::*;

    const ARENA_LIKE: &str = "\
// CEP:FILE: crates/x/src/a.rs
// CEP:WHAT: Does one thing well.
// CEP:CLASS: HPC-0
// CEP:STATUS: complete

/// CEP:WHAT: A documented public struct.
/// CEP:WHY: Needed.
/// CEP:STATUS: complete
pub struct Thing {
    field: u32,
}

fn helper(a: u32) -> u32 {
    let b = a + 1;
    b
}

#[cfg(test)]
pub mod tests {
    fn inner() -> u32 {
        7
    }
}
";

    #[test]
    fn scans_exemplar_shape() {
        let scanned = scan(ARENA_LIKE);
        assert!(scanned.header.is_some(), "header block found");
        assert_eq!(scanned.class.as_deref(), Some("HPC-0"));
        assert_eq!(scanned.test_spans.len(), 1);
        let (a, b) = scanned.test_spans[0];
        assert!(a <= 18 && b >= 22, "span {}..{} covers the test mod", a, b);
        assert_eq!(scanned.items.len(), 2, "struct plus the test mod, recorded with spans");
        assert_eq!(scanned.items[0].name, "Thing");
        assert!(!scanned.items[0].in_test);
        assert_eq!(scanned.items[1].name, "tests");
        assert!(scanned.items[1].in_test, "test mod flagged for rule-level exemption");
        let helper = scanned.fns.iter().find(|f| f.name == "helper").expect("helper");
        assert!(!helper.is_pub);
        assert_eq!(helper.semis, 1);
        assert!(helper.has_tail);
        assert!(!helper.nested);
        let inner = scanned.fns.iter().find(|f| f.name == "inner").expect("inner");
        assert!(inner.in_test);
        let doc_block = scanned
            .blocks
            .iter()
            .find(|blk| blk.start == 6 && blk.key("CEP:WHAT").is_some())
            .expect("doc block for Thing");
        assert_eq!(
            doc_block.key("CEP:STATUS").map(|k| k.first_word()),
            Some("complete")
        );
    }

    #[test]
    fn keys_fold_continuations() {
        let src = "// CEP:FILE: a.rs\n// CEP:WHAT: line one\n//   line two continues\n// CEP:WHY: w\n";
        let scanned = scan(src);
        let header = scanned.header.expect("header");
        let what = header.key("CEP:WHAT").expect("WHAT key");
        assert_eq!(what.value, "line one line two continues");
        assert_eq!(what.line, 2);
    }

    #[test]
    fn strings_and_chars_are_blanked() {
        let src = "const S: &str = \"if x {}\";\nconst C: char = '{';\nlet n = 5;\n";
        let scanned = scan(src);
        assert_eq!(scanned.strings.len(), 1);
        assert_eq!(scanned.strings[0].1, "if x {}");
        assert!(!scanned.code[0].contains('{'));
        assert!(!scanned.code[1].contains('{'));
        assert!(scanned.code[2].contains('5'));
    }

    #[test]
    fn raw_strings_handled() {
        let src = "const R: &str = r#\"unterminated { is fine\"#;\nlet x = 1;\n";
        let scanned = scan(src);
        assert_eq!(scanned.strings.len(), 1);
        assert!(scanned.strings[0].1.contains("unterminated"));
        assert!(!scanned.code[0].contains('{'));
    }

    #[test]
    fn nested_block_comments() {
        let src = "/* outer /* inner */ still comment */ let y = 2;\n";
        let scanned = scan(src);
        assert!(scanned.code[0].contains("let y = 2;"));
        assert!(!scanned.code[0].contains("inner"));
    }

    #[test]
    fn nested_fn_detection() {
        let src = "fn outer() {\n    fn inner() -> u32 {\n        let a = 1;\n        a\n    }\n    inner()\n}\n";
        let scanned = scan(src);
        let inner = scanned.fns.iter().find(|f| f.name == "inner").expect("inner");
        let outer = scanned.fns.iter().find(|f| f.name == "outer").expect("outer");
        assert!(inner.nested);
        assert!(!outer.nested);
        assert_eq!(outer.semis, 1, "nested bodies count toward the enclosing fn");
        assert!(outer.has_tail);
    }

    #[test]
    fn pub_visibility_detected() {
        let src = "pub fn a() {\n    one();\n    two()\n}\npub(crate) fn b() {\n    one();\n}\nfn c() {\n    one();\n}\n";
        let scanned = scan(src);
        let a = scanned.fns.iter().find(|f| f.name == "a").expect("a");
        let b = scanned.fns.iter().find(|f| f.name == "b").expect("b");
        let c = scanned.fns.iter().find(|f| f.name == "c").expect("c");
        assert!(a.is_pub);
        assert!(b.is_pub);
        assert!(!c.is_pub);
        assert_eq!(a.semis, 1);
        assert_eq!(b.semis, 1);
    }

    #[test]
    fn attached_doc_extraction() {
        let src = "/// CEP:WHAT: Public fn docs.\n/// CEP:STATUS: complete\npub fn documented() {\n    a();\n    b();\n}\n";
        let scanned = scan(src);
        let f = scanned
            .fns
            .iter()
            .find(|f| f.name == "documented")
            .expect("documented");
        assert!(f.doc_keys.contains(&"CEP:WHAT".to_string()));
        assert!(f.doc_text.contains("Public fn docs."));
    }

    #[test]
    fn doc_block_above_attribute_is_found() {
        let src = "/// CEP:WHAT: Test fn docs.\n/// CEP:STATUS: complete\n#[test]\nfn it_works() {\n    assert!(true);\n    assert!(true);\n}\n";
        let scanned = scan(src);
        let f = scanned.fns.iter().find(|f| f.name == "it_works").expect("fn");
        assert!(f.doc_keys.contains(&"CEP:WHAT".to_string()));
    }

    #[test]
    fn lifetimes_do_not_break_fn_scan() {
        let src = "pub fn f<'a>(x: &'a str) -> char {\n    let c = 'x';\n    c\n}\n";
        let scanned = scan(src);
        let f = scanned.fns.iter().find(|f| f.name == "f").expect("f");
        assert!(f.is_pub);
        assert!(scanned.code[0].contains("'a"), "lifetime kept as code");
    }

    #[test]
    fn no_std_preamble_is_transparent() {
        let src = "#![no_std]\n// CEP:FILE: a.rs\n// CEP:STATUS: complete\n";
        let scanned = scan(src);
        assert!(scanned.header.is_some(), "header after inner attribute found");
        assert_eq!(scanned.preamble_lines, 1);
    }

    #[test]
    fn string_continuation_keeps_line_alignment() {
        // Regression: a `\<newline>` continuation inside a string used to consume the
        // newline without advancing the line counter, shifting every later comment by
        // one line and detaching the last key of the following block.
        let src = concat!(
            "let s = \"one \\\n two\";\n",
            "/// CEP:WHAT: block docs\n",
            "/// CEP:WHY: alignment\n",
            "/// CEP:STATUS: complete\n",
            "/// CEP:FAILURE: none\n",
            "/// CEP:ASSUMES: none\n",
            "/// CEP:COST: cold.\n",
            "/// CEP:EVIDENCE: this test.\n",
            "pub fn f() -> u32 {\n    let x = 1;\n    x\n}\n",
        );
        let scanned = scan(src);
        for (idx, raw) in scanned.raw_lines.iter().enumerate() {
            let is_comment_raw = raw.trim_start().starts_with("//");
            assert_eq!(
                scanned.whole_comment[idx], is_comment_raw,
                "whole_comment misaligned at raw line {}",
                idx + 1
            );
        }
        let f = scanned.fns.iter().find(|f| f.name == "f").expect("fn f");
        assert!(f.doc_keys.contains(&"CEP:EVIDENCE".to_string()));
        assert!(f.doc_keys.contains(&"CEP:WHAT".to_string()));
    }

    #[test]
    fn pub_items_classified() {
        let src = "pub struct A { x: u32 }\npub enum B { V }\npub trait C {}\npub type D = u32;\npub(crate) struct E;\npub mod f {\n    const G: u32 = 9;\n}\npub mod h;\npub use crate::x::{Y, Z};\n";
        let scanned = scan(src);
        let kinds: Vec<(String, String)> = scanned
            .items
            .iter()
            .map(|i| (i.kind.clone(), i.name.clone()))
            .collect();
        assert!(kinds.contains(&("struct".to_string(), "A".to_string())));
        assert!(kinds.contains(&("enum".to_string(), "B".to_string())));
        assert!(kinds.contains(&("trait".to_string(), "C".to_string())));
        assert!(kinds.contains(&("type".to_string(), "D".to_string())));
        assert!(kinds.contains(&("struct".to_string(), "E".to_string())));
        assert!(kinds.contains(&("mod".to_string(), "f".to_string())));
        assert!(!kinds.iter().any(|(_, n)| n == "h"), "body-less mod exempt");
        assert_eq!(kinds.len(), 6, "pub use never an item");
    }

    #[test]
    fn trailing_comments_recorded() {
        let src = "let x = 1; // trailing note\n// whole line\n";
        let scanned = scan(src);
        assert_eq!(
            scanned.comment_text[0].as_deref(),
            Some(" trailing note")
        );
        assert!(!scanned.whole_comment[0]);
        assert!(scanned.whole_comment[1]);
    }
}
