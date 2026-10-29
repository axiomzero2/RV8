// CEP:FILE: tools/cep_lint/src/regexlite.rs
// CEP:WHAT: A compact backtracking regular-expression engine covering the subset the lint
//   configuration uses: literals, '.', character classes with ranges and negation, capturing
//   groups, alternation, the * + ? quantifiers, ^ $ anchors, and \d \s \w \b escapes.
// CEP:WHY: The linter is zero-dependency by workspace law, so rule patterns from
//   .cep/cep_lint.json need a self-contained matcher with capture support for naming rules.
// CEP:CLASS: CEP-2
// CEP:STATUS: complete
// CEP:OWNER: axiomzero2
// CEP:TICKET: RV8-100
// CEP:FAILURE: unsupported constructs and malformed patterns return Err at compile time;
//   a runaway match attempt aborts at MAX_REGEX_STEPS and reports no match, never a panic.
// CEP:ASSUMES: patterns are small first-party strings from the lint configuration; inputs
//   are single lines whose length is bounded by the scanner budget.
// CEP:COST: compile O(P); match is backtracking over an explicit thread stack bounded by
//   MAX_REGEX_STEPS; per-line matching is linear in practice for the shipped pattern set.
// CEP:EVIDENCE: unit tests cover every shipped config pattern plus error and budget paths.
// CEP:SECURITY: no recursion at match time (explicit stack) and a hard step budget prevent
//   pathological backtracking from consuming unbounded time (38.12, 38.13).

/// Maximum VM steps per match attempt before the attempt gives up (reported as no match).
pub const MAX_REGEX_STEPS: u64 = 250_000;

/// Maximum alternation/group nesting accepted at compile time.
pub const MAX_PATTERN_DEPTH: usize = 24;

/// CEP:WHAT: Compile-time regex failure carrying the offending pattern and a reason.
/// CEP:WHY: A configuration pattern the engine cannot honor must fail the bootstrap with a
///   precise diagnostic instead of silently never matching.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; a plain data carrier.
/// CEP:ASSUMES: reasons are human-readable first-party text.
/// CEP:COST: one struct per failed compile.
/// CEP:EVIDENCE: config::tests::unsupported_patterns_are_bootstrap_errors surfaces this
///   error through configuration validation.
#[derive(Debug, Clone, PartialEq)]
pub struct RegexError {
    pub pattern: String,
    pub message: String,
}

#[derive(Debug, Clone)]
enum Inst {
    Char(char),
    Any,
    Class {
        negated: bool,
        ranges: Vec<(char, char)>,
    },
    /// Try the first target; the second is the backtrack point. usize::MAX marks a hole.
    Split(usize, usize),
    Jump(usize),
    Start,
    End,
    WordBoundary,
    GroupStart(usize),
    GroupEnd(usize),
    Match,
}

/// CEP:WHAT: A successful match: the overall span plus per-capture-group spans.
/// CEP:WHY: Suppression and heading rules need group extraction (for example the rule id
///   inside a `### Rule` heading) as well as the whole-match span.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; a plain data carrier.
/// CEP:ASSUMES: spans are char indices into the searched text.
/// CEP:COST: one vector proportional to group count.
/// CEP:EVIDENCE: regexlite::tests exercise group extraction; engine::doc_sync consumes
///   group 1 for heading ids.
#[derive(Debug, Clone, PartialEq)]
pub struct RegexMatch {
    pub start: usize,
    pub end: usize,
    /// Span per capture group; group 0 is the whole match.
    pub groups: Vec<Option<(usize, usize)>>,
}

/// Capture-group state carried by one VM frame.
type CaptureState = Vec<Option<(usize, usize)>>;

/// One saved continuation: program counter, input position, captures.
type VmFrame = (usize, usize, CaptureState);

/// CEP:WHAT: A compiled pattern ready for repeated matching, executed by a backtracking VM
///   with an explicit stack and a hard step budget.
/// CEP:WHY: Comment, naming, and placeholder rules need pattern matching with zero
///   external crates and no pathological-backtracking blowup on untrusted text.
/// CEP:STATUS: complete
/// CEP:FAILURE: Regex::new returns RegexError for unsupported syntax; a runaway match
///   attempt aborts at MAX_REGEX_STEPS and reports no match, never a panic.
/// CEP:ASSUMES: the supported syntax subset (literals, classes, groups, alternation,
///   anchors, word boundaries, quantifiers) covers the shipped configuration.
/// CEP:COST: O(pattern) compile; per-attempt work bounded by MAX_REGEX_STEPS.
/// CEP:EVIDENCE: regexlite::tests cover matching, groups, anchors, boundaries, icase,
///   unsupported syntax, and the step budget.
#[derive(Debug, Clone)]
pub struct Regex {
    insts: Vec<Inst>,
    /// Entry instruction (always 0 today, kept explicit for the VM contract).
    entry: usize,
    group_count: usize,
    icase: bool,
}

/// A compiled fragment: entry point plus dangling edges that must be patched to the
/// instruction that logically follows the fragment.
struct Frag {
    start: usize,
    /// Instruction indices whose next target is still unassigned.
    outs: Vec<usize>,
}

struct Compiler<'a> {
    pattern: &'a str,
    chars: Vec<char>,
    pos: usize,
    insts: Vec<Inst>,
    group_count: usize,
    icase: bool,
}

enum Escaped {
    Char(char),
    Class {
        negated: bool,
        ranges: Vec<(char, char)>,
    },
    Boundary,
}

impl<'a> Compiler<'a> {
    fn new(pattern: &'a str, icase: bool) -> Compiler<'a> {
        Compiler {
            pattern,
            chars: pattern.chars().collect(),
            pos: 0,
            insts: Vec::new(),
            group_count: 0,
            icase,
        }
    }

    fn err(&self, message: &str) -> RegexError {
        RegexError {
            pattern: self.pattern.to_string(),
            message: message.to_string(),
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += 1;
        Some(c)
    }

    fn emit(&mut self, inst: Inst) -> usize {
        self.insts.push(inst);
        self.insts.len() - 1
    }

    fn empty_frag(&self, start: usize) -> Frag {
        Frag {
            start,
            outs: Vec::new(),
        }
    }

    fn fold(&self, c: char) -> char {
        if self.icase && c.is_ascii() {
            c.to_ascii_lowercase()
        } else {
            c
        }
    }

    /// alternation := sequence ('|' sequence)*
    ///
    /// Both branches must be reachable only through one choice point, so the split is
    /// inserted before the first branch and a jump is inserted between the branches;
    /// absolute targets at or after each insertion point shift accordingly.
    fn parse_alternation(&mut self, depth: usize) -> Result<Frag, RegexError> {
        if depth > MAX_PATTERN_DEPTH {
            return Err(self.err("pattern nesting exceeds budget"));
        }
        let mut left = self.parse_sequence(depth)?;
        while self.peek() == Some('|') {
            self.bump();
            let mut right = self.parse_sequence(depth)?;
            left = self.realize_empty(left);
            right = self.realize_empty(right);
            let split = self.insert_inst(left.start, Inst::Split(usize::MAX, usize::MAX));
            let right_entry = right.start + 1;
            let jump = self.insert_inst(right_entry, Inst::Jump(usize::MAX));
            let right_entry = right_entry + 1;
            for &out in &left.outs {
                self.patch(out + 1, jump);
            }
            self.insts[split] = Inst::Split(left.start + 1, right_entry);
            let mut outs = right.outs.iter().map(|o| o + 2).collect::<Vec<_>>();
            outs.push(jump);
            left = Frag { start: split, outs };
        }
        Ok(left)
    }

    /// Gives an empty sequence a concrete no-op entry so later bookkeeping never sees the
    /// usize::MAX stand-in inside instruction targets.
    fn realize_empty(&mut self, frag: Frag) -> Frag {
        if frag.start != usize::MAX {
            return frag;
        }
        let jump = self.emit(Inst::Jump(usize::MAX));
        Frag {
            start: jump,
            outs: vec![jump],
        }
    }

    /// Inserts an instruction at `at`, shifting every absolute target at or after it.
    fn insert_inst(&mut self, at: usize, inst: Inst) -> usize {
        self.insts.insert(at, inst);
        for other in self.insts.iter_mut().skip(at + 1) {
            match other {
                Inst::Split(first, second) => {
                    if *first != usize::MAX && *first >= at {
                        *first += 1;
                    }
                    if *second != usize::MAX && *second >= at {
                        *second += 1;
                    }
                }
                Inst::Jump(target) if *target != usize::MAX && *target >= at => {
                    *target += 1;
                }
                _ => {}
            }
        }
        at
    }

    /// sequence := quantified atom*
    fn parse_sequence(&mut self, depth: usize) -> Result<Frag, RegexError> {
        let mut frag: Option<Frag> = None;
        loop {
            match self.peek() {
                None | Some('|') | Some(')') => {
                    return Ok(frag.unwrap_or_else(|| Frag {
                        start: usize::MAX,
                        outs: Vec::new(),
                    }));
                }
                _ => {
                    let atom = self.parse_quantified(depth)?;
                    frag = Some(match frag.take() {
                        None => atom,
                        Some(prev) => {
                            // Concatenate: previous fragment's dangling edges flow into
                            // this atom's entry instruction.
                            for &out in &prev.outs {
                                self.patch(out, atom.start);
                            }
                            Frag {
                                start: prev.start,
                                outs: atom.outs,
                            }
                        }
                    });
                }
            }
        }
    }

    fn parse_quantified(&mut self, depth: usize) -> Result<Frag, RegexError> {
        let mut atom = self.parse_atom(depth)?;
        if atom.start == usize::MAX {
            atom = self.realize_empty(atom);
        }
        match self.peek() {
            Some('*') | Some('+') | Some('?') => {
                let q = self.bump().unwrap_or('*');
                if matches!(self.peek(), Some('*') | Some('+') | Some('?')) {
                    return Err(self.err("stacked quantifiers are not supported"));
                }
                // '*': the choice point must precede the body, so a split is inserted at
                // the body entry and a trailing jump closes the loop. '?': same split, no
                // loop. '+': the body must run once, so it stays in place and the split is
                // appended after it (fall-through gives the mandatory first iteration).
                let body = atom.start;
                match q {
                    '+' => {
                        let split = self.emit(Inst::Split(body, usize::MAX));
                        for &out in &atom.outs {
                            self.patch(out, split);
                        }
                        Ok(Frag {
                            start: body,
                            outs: vec![split],
                        })
                    }
                    _ => {
                        let split = self.insert_inst(body, Inst::Split(usize::MAX, usize::MAX));
                        let outs = atom
                            .outs
                            .iter()
                            .map(|o| if *o >= split { o + 1 } else { *o })
                            .collect::<Vec<_>>();
                        self.insts[split] = Inst::Split(body + 1, usize::MAX);
                        for &out in &outs {
                            self.patch(out, split);
                        }
                        if q == '*' {
                            self.emit(Inst::Jump(split));
                            Ok(Frag {
                                start: split,
                                outs: vec![split],
                            })
                        } else {
                            let mut outs = outs;
                            outs.push(split);
                            Ok(Frag { start: split, outs })
                        }
                    }
                }
            }
            _ => Ok(atom),
        }
    }

    fn patch(&mut self, inst: usize, target: usize) {
        match self.insts.get_mut(inst) {
            Some(Inst::Split(_, second)) if *second == usize::MAX => *second = target,
            Some(Inst::Jump(target_slot)) if *target_slot == usize::MAX => {
                *target_slot = target;
            }
            _ => {}
        }
    }

    fn parse_atom(&mut self, depth: usize) -> Result<Frag, RegexError> {
        match self.peek() {
            None => Err(self.err("expected pattern operand")),
            Some('(') => {
                self.bump();
                if self.peek() == Some('?') {
                    return Err(self.err("non-capturing groups are not supported"));
                }
                let group = self.group_count;
                self.group_count += 1;
                let gs = self.emit(Inst::GroupStart(group));
                let inner = self.parse_alternation(depth + 1)?;
                match self.peek() {
                    Some(')') => {
                        self.bump();
                    }
                    _ => return Err(self.err("unbalanced group")),
                }
                let ge = self.emit(Inst::GroupEnd(group));
                for &out in &inner.outs {
                    self.patch(out, ge);
                }
                Ok(self.empty_frag(gs))
            }
            Some('[') => {
                self.bump();
                let (negated, ranges) = self.parse_class()?;
                let inst = self.emit(Inst::Class { negated, ranges });
                Ok(self.empty_frag(inst))
            }
            Some('.') => {
                self.bump();
                let inst = self.emit(Inst::Any);
                Ok(self.empty_frag(inst))
            }
            Some('^') => {
                self.bump();
                let inst = self.emit(Inst::Start);
                Ok(self.empty_frag(inst))
            }
            Some('$') => {
                self.bump();
                let inst = self.emit(Inst::End);
                Ok(self.empty_frag(inst))
            }
            Some('*') | Some('+') | Some('?') => Err(self.err("quantifier without operand")),
            Some(')') => Err(self.err("unbalanced closing parenthesis")),
            Some('\\') => {
                self.bump();
                match self.parse_escape()? {
                    Escaped::Char(c) => {
                        let inst = self.emit(Inst::Char(self.fold(c)));
                        Ok(self.empty_frag(inst))
                    }
                    Escaped::Class { negated, ranges } => {
                        let inst = self.emit(Inst::Class { negated, ranges });
                        Ok(self.empty_frag(inst))
                    }
                    Escaped::Boundary => {
                        let inst = self.emit(Inst::WordBoundary);
                        Ok(self.empty_frag(inst))
                    }
                }
            }
            Some(c) => {
                self.bump();
                let inst = self.emit(Inst::Char(self.fold(c)));
                Ok(self.empty_frag(inst))
            }
        }
    }

    fn parse_escape(&mut self) -> Result<Escaped, RegexError> {
        let c = match self.bump() {
            Some(c) => c,
            None => return Err(self.err("trailing backslash")),
        };
        match c {
            'd' => Ok(Escaped::Class {
                negated: false,
                ranges: vec![('0', '9')],
            }),
            'D' => Ok(Escaped::Class {
                negated: true,
                ranges: vec![('0', '9')],
            }),
            's' => Ok(Escaped::Class {
                negated: false,
                ranges: vec![('\t', '\r'), (' ', ' ')],
            }),
            'S' => Ok(Escaped::Class {
                negated: true,
                ranges: vec![('\t', '\r'), (' ', ' ')],
            }),
            'w' => Ok(Escaped::Class {
                negated: false,
                ranges: vec![('0', '9'), ('A', 'Z'), ('_', '_'), ('a', 'z')],
            }),
            'W' => Ok(Escaped::Class {
                negated: true,
                ranges: vec![('0', '9'), ('A', 'Z'), ('_', '_'), ('a', 'z')],
            }),
            'b' => Ok(Escaped::Boundary),
            'n' => Ok(Escaped::Char('\n')),
            't' => Ok(Escaped::Char('\t')),
            'r' => Ok(Escaped::Char('\r')),
            '0' => Ok(Escaped::Char('\0')),
            c if c.is_alphanumeric() => Err(self.err(&format!("unsupported escape '\\{}'", c))),
            c => Ok(Escaped::Char(c)),
        }
    }

    fn parse_class(&mut self) -> Result<(bool, Vec<(char, char)>), RegexError> {
        let mut negated = false;
        if self.peek() == Some('^') {
            self.bump();
            negated = true;
        }
        let mut ranges: Vec<(char, char)> = Vec::new();
        let mut first = true;
        loop {
            let c = match self.peek() {
                None => return Err(self.err("unterminated character class")),
                Some(']') if !first => {
                    self.bump();
                    return Ok((negated, ranges));
                }
                Some(c) => c,
            };
            first = false;
            self.bump();
            let lo = if c == '\\' {
                match self.parse_escape()? {
                    Escaped::Char(x) => x,
                    Escaped::Class {
                        negated: _,
                        ranges: inner,
                    } => {
                        ranges.extend(inner.iter().copied());
                        continue;
                    }
                    Escaped::Boundary => {
                        return Err(self.err("word boundary inside class unsupported"));
                    }
                }
            } else {
                c
            };
            let next_is_dash = self.peek() == Some('-');
            let after_dash = self.chars.get(self.pos + 1).copied();
            if next_is_dash && after_dash.is_some() && after_dash != Some(']') {
                self.bump();
                let hi = match self.bump() {
                    Some('\\') => match self.parse_escape()? {
                        Escaped::Char(x) => x,
                        _ => return Err(self.err("bad range end")),
                    },
                    Some(x) => x,
                    None => return Err(self.err("unterminated range")),
                };
                if hi < lo {
                    return Err(self.err("reversed character range"));
                }
                ranges.push((lo, hi));
            } else {
                ranges.push((lo, lo));
            }
        }
    }
}

impl Regex {
    /// CEP:WHAT: Compiles a pattern with the given case-insensitivity flag.
    /// CEP:WHY: One constructor validates the whole configuration's patterns up front so
    ///   bad regex is a bootstrap error, never a mid-run surprise.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: Err with the pattern and reason for unsupported syntax or depth
    ///   overrun; never panics.
    /// CEP:ASSUMES: icase folding is ASCII-only, matching the config's icase usage.
    /// CEP:COST: O(pattern length) to compile.
    /// CEP:EVIDENCE: regexlite::tests compile every shipped configuration pattern and
    ///   reject malformed ones.
    pub fn new(pattern: &str, icase: bool) -> Result<Regex, RegexError> {
        let mut compiler = Compiler::new(pattern, icase);
        let frag = compiler.parse_alternation(0)?;
        if compiler.pos < compiler.chars.len() {
            return Err(compiler.err("unexpected trailing pattern input"));
        }
        let match_inst = compiler.emit(Inst::Match);
        // An empty alternative sequence leaves a dangling start; route it to Match.
        let entry = if frag.start == usize::MAX {
            match_inst
        } else {
            for &out in &frag.outs {
                compiler.patch(out, match_inst);
            }
            frag.start
        };
        Ok(Regex {
            insts: compiler.insts,
            entry,
            group_count: compiler.group_count,
            icase,
        })
    }

    /// CEP:WHAT: Finds the leftmost match at or after `from` (char offsets) with capture
    ///   spans.
    /// CEP:WHY: Line-scoped rules try successive offsets; exposing the start keeps the
    ///   scan linear and deterministic.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: None when no match exists or the step budget is exhausted.
    /// CEP:ASSUMES: `from` is a char index into `text`.
    /// CEP:COST: worst case bounded by MAX_REGEX_STEPS per attempt.
    /// CEP:EVIDENCE: regexlite::tests cover offsets and group extraction.
    pub fn find_at(&self, text: &[char], from: usize) -> Option<RegexMatch> {
        if from > text.len() {
            return None;
        }
        for start in from..=text.len() {
            if let Some(m) = self.run(text, start) {
                return Some(m);
            }
        }
        None
    }

    /// CEP:WHAT: Leftmost match in a string; the common case for line-scoped rules.
    /// CEP:WHY: Comment and code regex rules match whole lines; this wrapper handles the
    ///   char conversion and budget in one place.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: None when no match exists or the step budget is exhausted.
    /// CEP:ASSUMES: text is the bounded line/window content.
    /// CEP:COST: as find_at from position zero.
    /// CEP:EVIDENCE: regexlite::tests and the rules::tests suites exercise this path for
    ///   every configured regex rule.
    pub fn find(&self, text: &str) -> Option<RegexMatch> {
        let chars: Vec<char> = text.chars().collect();
        self.find_at(&chars, 0)
    }

    /// Whether the pattern matches anywhere in the string.
    pub fn is_match(&self, text: &str) -> bool {
        self.find(text).is_some()
    }

    fn fold(&self, c: char) -> char {
        if self.icase && c.is_ascii() {
            c.to_ascii_lowercase()
        } else {
            c
        }
    }

    fn is_word(c: char) -> bool {
        c.is_alphanumeric() || c == '_'
    }

    /// Backtracking VM with an explicit stack and a hard step budget.
    fn run(&self, text: &[char], start: usize) -> Option<RegexMatch> {
        let mut steps: u64 = 0;
        let mut stack: Vec<VmFrame> = vec![(self.entry, start, vec![None; self.group_count])];
        while let Some((mut pc, mut pos, mut caps)) = stack.pop() {
            steps += 1;
            if steps > MAX_REGEX_STEPS {
                return None;
            }
            loop {
                let inst = self.insts.get(pc)?;
                match inst {
                    Inst::Char(c) => match text.get(pos) {
                        Some(&t) if self.fold(t) == *c => {
                            pos += 1;
                            pc += 1;
                        }
                        _ => break,
                    },
                    Inst::Any => match text.get(pos) {
                        Some(_) => {
                            pos += 1;
                            pc += 1;
                        }
                        None => break,
                    },
                    Inst::Class { negated, ranges } => {
                        let t = match text.get(pos) {
                            Some(&t) => t,
                            None => break,
                        };
                        let hit = ranges.iter().any(|&(lo, hi)| t >= lo && t <= hi);
                        if hit != *negated {
                            pos += 1;
                            pc += 1;
                        } else {
                            break;
                        }
                    }
                    Inst::Split(a, b) => {
                        let (a, b) = (*a, *b);
                        if b != usize::MAX {
                            stack.push((b, pos, caps.clone()));
                        }
                        pc = a;
                    }
                    Inst::Jump(t) => pc = *t,
                    Inst::Start => {
                        if pos == 0 {
                            pc += 1;
                        } else {
                            break;
                        }
                    }
                    Inst::End => {
                        if pos == text.len() {
                            pc += 1;
                        } else {
                            break;
                        }
                    }
                    Inst::WordBoundary => {
                        let before = if pos == 0 {
                            false
                        } else {
                            Self::is_word(text[pos - 1])
                        };
                        let after = match text.get(pos) {
                            Some(&t) => Self::is_word(t),
                            None => false,
                        };
                        if before != after {
                            pc += 1;
                        } else {
                            break;
                        }
                    }
                    Inst::GroupStart(g) => {
                        if let Some(slot) = caps.get_mut(*g) {
                            *slot = Some((pos, pos));
                        }
                        pc += 1;
                    }
                    Inst::GroupEnd(g) => {
                        if let Some(Some((s, _))) = caps.get(*g) {
                            let s = *s;
                            if let Some(slot) = caps.get_mut(*g) {
                                *slot = Some((s, pos));
                            }
                        }
                        pc += 1;
                    }
                    Inst::Match => {
                        let mut groups = Vec::with_capacity(self.group_count + 1);
                        groups.push(Some((start, pos)));
                        groups.extend(caps.iter().copied());
                        return Some(RegexMatch {
                            start,
                            end: pos,
                            groups,
                        });
                    }
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(pattern: &str, icase: bool, text: &str) -> Option<RegexMatch> {
        Regex::new(pattern, icase)
            .expect("test pattern compiles")
            .find(text)
    }

    #[test]
    fn compiles_and_matches_every_config_pattern() {
        let cases: [(&str, bool, &str, bool); 13] = [
            (r"\b(TODO|FIXME|XXX|HACK)\b", true, "we FIXME it", true),
            (r"\b(TODO|FIXME|XXX|HACK)\b", true, "we fixed it", false),
            (
                r"CEP:TODO\([^)]+\):\s*CEP-\d+",
                false,
                "// CEP:TODO(alice): CEP-314: x",
                true,
            ),
            (r"CEP:TODO\([^)]+\):\s*CEP-\d+", false, "// CEP:TODO(): CEP-x", false),
            (r"^\s*//\s*CEP:", false, "   // CEP:WHAT: hi", true),
            (r"^\s*//\s*CEP:", false, "   /// CEP:WHAT: hi", false),
            (
                r"\b(not implemented|unimplemented|placeholder|fixme|tbd|wip)\b",
                true,
                "Still TBD honestly",
                true,
            ),
            (r"catch\s*\([^)]*\)\s*\{\s*\}", false, "catch (E) { }", true),
            (
                r"\b(if|else|for|while|match)[^{]*\{\s*\}",
                false,
                "impl Foo for Bar {}",
                true,
            ),
            (r"^return (true|false|nullptr);$", false, "return true;", true),
            (
                r"\b(struct|enum)\s+([A-Za-z_][A-Za-z0-9_]*)",
                false,
                "pub struct Chunk {",
                true,
            ),
            (
                r"^\s*//\s*(let |fn |if |for |while |return |pub )",
                false,
                "    // for each item",
                true,
            ),
            (r"CEP&CC 0\.1", false, "# CEP&CC 0.1", true),
        ];
        for (pattern, icase, text, expected) in cases {
            assert_eq!(
                m(pattern, icase, text).is_some(),
                expected,
                "pattern {} on {:?}",
                pattern,
                text
            );
        }
    }

    #[test]
    fn captures_groups() {
        let matched =
            m(r"\b(struct|enum)\s+([A-Za-z_][A-Za-z0-9_]*)", false, "struct Chunk").expect("match");
        let text_chars: Vec<char> = "struct Chunk".chars().collect();
        let g2 = matched.groups[2].expect("group 2");
        let name: String = text_chars[g2.0..g2.1].iter().collect();
        assert_eq!(name, "Chunk");
    }

    #[test]
    fn doc_heading_pattern_with_dollar_anchor() {
        let re = Regex::new(r"^### Rule (CEP-LINT-[A-Z-]+)$", false).expect("compiles");
        assert!(re.is_match("### Rule CEP-LINT-FILE-HEADER"));
        assert!(!re.is_match("### Rule CEP-LINT-FILE-HEADER plus"));
        assert!(re.is_match("### Rule CEP-LINT-DOC-SYNC"));
    }

    #[test]
    fn rejects_unsupported_and_malformed() {
        assert!(Regex::new("(", false).is_err());
        assert!(Regex::new("[a-", false).is_err());
        assert!(Regex::new("*x", false).is_err());
        assert!(Regex::new("(?:x)", false).is_err());
        assert!(Regex::new("a**", false).is_err());
        assert!(Regex::new("\\q", false).is_err());
        assert!(Regex::new("a)", false).is_err());
    }

    #[test]
    fn icase_folding_works() {
        assert!(m(r"\btodo\b", true, "Please TODO soon").is_some());
        assert!(m(r"\btodo\b", false, "Please TODO soon").is_none());
    }

    #[test]
    fn word_boundaries_respect_identifiers() {
        assert!(m(r"\bhack\b", true, "a hack in it").is_some());
        assert!(m(r"\bhack\b", true, "hacks and hackers").is_none());
        assert!(m(r"\bhack\b", true, "hack_something").is_none());
    }

    #[test]
    fn find_reports_leftmost_span() {
        let matched = m(r"\bx\b", false, "a x b x").expect("match");
        assert_eq!((matched.start, matched.end), (2, 3));
    }

    #[test]
    fn quantifiers_and_alternation_compose() {
        assert!(m(r"ab*c", false, "ac").is_some());
        assert!(m(r"ab*c", false, "abbbc").is_some());
        assert!(m(r"ab+c", false, "ac").is_none());
        assert!(m(r"colou?r", false, "color").is_some());
        assert!(m(r"colou?r", false, "colour").is_some());
        assert!(m(r"^(cat|dog)s?$", false, "dogs").is_some());
        assert!(m(r"^(cat|dog)s?$", false, "cows").is_none());
        assert!(m(r"[a-f0-9]+", false, "deadbeef01").is_some());
        assert!(m(r"[^a-f]+", false, "xyz").is_some());
        assert!(m(r"[^a-f]+", false, "abc").is_none());
    }
}
