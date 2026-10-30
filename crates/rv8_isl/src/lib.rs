// CEP:FILE: crates/rv8_isl/src/lib.rs
// CEP:WHAT: ISL, the Instruction-Set description Language: a declarative target
//   description format. A target is DATA - register lanes per class (names, hardware
//   encodings, ABI roles, clobber roles), per-class move and spill costs, the ABI
//   return and argument registers, and the frame shape. This crate defines the text
//   grammar, an iterative fail-closed parser, structural validation, and the
//   TargetDescription model that the shared register allocator (one reggolloc) and
//   the cross-check tests consume.
// CEP:WHY: STANDARDS law 7 (no hard-coded assumptions): target facts must be named,
//   justified configuration, not code paths. One allocator core serves every target;
//   adding a target means writing one ISL file, not forking the backend.
// CEP:CLASS: CEP-1
// CEP:STATUS: complete
// CEP:FAILURE: IslError names the offending construct; the parser never panics, never
//   recurses over input (a bracket-depth budget replaces recursion), and every
//   fallible path returns Result.
// CEP:ASSUMES: lane order within a class is the class-local RegId order; `enc` is the
//   machine physical-register encoding; at most 64 lanes per class (barrier bitmasks).
// CEP:COST: parsing is O(text); validation O(lanes).
// CEP:EVIDENCE: rv8_isl::tests (grammar and rejection corpus) and the cross-check
//   tests in rv8_backend::desc that pin x86-64.isl against the hard-coded tables.
use std::fmt;

/// CEP:WHAT: The ISL parse and validation error.
/// CEP:WHY: Fail-closed diagnostics with the offending token.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: POD plus one String.
/// CEP:EVIDENCE: rejection corpus in this module.
#[derive(Clone, Debug, PartialEq)]
pub enum IslError {
    /// Lexical failure with the offending byte offset.
    Lex { at: usize },
    /// Unexpected token with the offset and what was expected.
    Syntax { at: usize, want: &'static str },
    /// Structural validation failure.
    Invalid { reason: String },
}

/// CEP:WHAT: Display for ISL errors.
/// CEP:WHY: Driver-grade diagnostics without Debug noise (closes part of RV8-135 for
/// this crate's own error type).
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(message).
/// CEP:EVIDENCE: display_is_human in this module.
impl fmt::Display for IslError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IslError::Lex { at } => write!(f, "isl: lexical error at byte {at}"),
            IslError::Syntax { at, want } => {
                write!(f, "isl: expected {want} at byte {at}")
            }
            IslError::Invalid { reason } => write!(f, "isl: {reason}"),
        }
    }
}

/// CEP:WHAT: One register lane: name, machine encoding, savedness, and roles.
/// CEP:WHY: The atom of a target description.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: enc is the machine physical-register number.
/// CEP:COST: POD.
/// CEP:EVIDENCE: cross-check tests.
#[derive(Clone, Debug, PartialEq)]
pub struct Lane {
    /// Register name as used in the description (lowercase).
    pub name: String,
    /// Machine physical-register encoding (e.g. machir numbering).
    pub enc: u16,
    /// True for callee-saved lanes.
    pub callee_saved: bool,
    /// The ABI return role, when present.
    pub is_return: bool,
    /// The zero-based ABI argument positions this lane serves.
    pub args: Vec<u8>,
    /// True when the divide sequence clobbers this lane.
    pub div_clobber: bool,
}

/// CEP:WHAT: One register class: lanes plus the cost model the allocator sees.
/// CEP:WHY: The allocator solves per class over these lanes.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: at most 64 lanes (bitmask barriers).
/// CEP:COST: small vector per class.
/// CEP:EVIDENCE: cross-check tests.
#[derive(Clone, Debug, PartialEq)]
pub struct Class {
    /// Class name ("gpr", "fpr", ...).
    pub name: String,
    /// Lanes in RegId order (class-local index = position here).
    pub lanes: Vec<Lane>,
    /// Cost of one register-to-register move (ISL units, scaled by the consumer).
    pub move_cost: f64,
    /// Cost of spilling one value of this class.
    pub spill_cost: f64,
}

/// CEP:WHAT: The frame shape facts.
/// CEP:WHY: The rewriter and the frame calculator consume alignment and red-zone
/// policy from the description.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: alignment is a power of two and at least 16 (machir verify minimum).
/// CEP:COST: POD.
/// CEP:EVIDENCE: validation in this module.
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    /// Stack alignment in bytes.
    pub alignment: u32,
    /// True when leaf frames may use the red zone without a stack adjustment.
    pub red_zone_leaf: bool,
}

/// CEP:WHAT: The whole target description: name, classes, ABI registers, frame.
/// CEP:WHY: THE target as data; the single input of the shared allocator.
/// CEP:STATUS: complete
/// CEP:FAILURE: construction only through parse, which validates.
/// CEP:ASSUMES: none
/// CEP:COST: owns small vectors.
/// CEP:EVIDENCE: cross-check tests.
#[derive(Clone, Debug, PartialEq)]
pub struct TargetDescription {
    /// Target name from the header.
    pub name: String,
    /// Register classes in declaration order.
    pub classes: Vec<Class>,
    /// The ABI return register (must be a lane of the first class).
    pub return_reg: String,
    /// The ABI argument registers in ABI order (lanes of the first class).
    pub args: Vec<String>,
    /// Frame shape.
    pub frame: Frame,
}

impl TargetDescription {
    /// CEP:WHAT: Looks a lane up by class name and register name.
    /// CEP:WHY: Cross-module resolution (ABI names to lanes).
    /// CEP:STATUS: complete
    /// CEP:FAILURE: None when either name is unknown.
    /// CEP:ASSUMES: none
    /// CEP:COST: O(lanes).
    /// CEP:EVIDENCE: lookup tests in this module.
    pub fn lane(&self, class: &str, reg: &str) -> Option<&Lane> {
        let c = self.classes.iter().find(|c| c.name == class)?;
        c.lanes.iter().find(|l| l.name == reg)
    }
}

/// CEP:WHAT: Maximum bracket nesting depth the parser accepts (38.12 named budget).
/// CEP:WHY: Iterative parsing with a depth budget replaces recursion over untrusted
/// input (38.6).
/// CEP:STATUS: complete
/// CEP:FAILURE: Syntax beyond the budget.
/// CEP:ASSUMES: descriptions are small, shallow documents.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: depth_rejected test in this module.
pub const MAX_ISL_DEPTH: usize = 16;

/// CEP:WHAT: Maximum description size in bytes (38.12 named budget).
/// CEP:WHY: Bounds parser work per document.
/// CEP:STATUS: complete
/// CEP:FAILURE: Lex error past the budget.
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: oversize_rejected test in this module.
pub const MAX_ISL_BYTES: usize = 1 << 16;

/// CEP:WHAT: One token: a word, a quoted string, a number, or punctuation, with its
/// byte offset for diagnostics.
/// CEP:WHY: The parser's currency.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: POD plus slice.
/// CEP:EVIDENCE: lexer tests.
#[derive(Clone, Debug, PartialEq)]
struct Token<'a> {
    kind: Tok<'a>,
    at: usize,
}

/// CEP:WHAT: Token kinds.
/// CEP:WHY: Grammar vocabulary.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: POD.
/// CEP:EVIDENCE: lexer tests.
#[derive(Clone, Debug, PartialEq)]
enum Tok<'a> {
    Word(&'a str),
    Str(&'a str),
    Num(f64),
    LBrace,
    RBrace,
}

/// CEP:WHAT: Tokenizes ISL text iteratively; '#' starts a line comment.
/// CEP:WHY: The parser's first stage; every byte offset is preserved for errors.
/// CEP:STATUS: complete
/// CEP:FAILURE: IslError::Lex on unterminated strings or invalid numerics.
/// CEP:ASSUMES: UTF-8 input.
/// CEP:COST: O(text).
/// CEP:EVIDENCE: lexer tests in this module.
fn lex(src: &str) -> Result<Vec<Token<'_>>, IslError> {
    let b = src.as_bytes();
    let mut toks = Vec::new();
    let mut i = 0usize;
    while i < b.len() {
        let c = b[i];
        match c {
            b'#' => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            b' ' | b'\t' | b'\n' | b'\r' => i += 1,
            b'{' => {
                toks.push(Token { kind: Tok::LBrace, at: i });
                i += 1;
            }
            b'}' => {
                toks.push(Token { kind: Tok::RBrace, at: i });
                i += 1;
            }
            b'"' => {
                let start = i;
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    i += 1;
                }
                if i >= b.len() {
                    return Err(IslError::Lex { at: start });
                }
                let text = &src[start + 1..i];
                toks.push(Token { kind: Tok::Str(text), at: start });
                i += 1;
            }
            _ if c.is_ascii_digit()
                || (c == b'-' && i + 1 < b.len() && b[i + 1].is_ascii_digit())
                || c == b'.' =>
            {
                let start = i;
                i += 1;
                while i < b.len()
                    && (b[i].is_ascii_digit() || b[i] == b'.' || b[i] == b'-')
                {
                    i += 1;
                }
                let text = &src[start..i];
                let n: f64 = text.parse().map_err(|_| IslError::Lex { at: start })?;
                toks.push(Token { kind: Tok::Num(n), at: start });
            }
            _ if c.is_ascii_alphanumeric() || c == b'_' || c == b'-' => {
                let start = i;
                while i < b.len()
                    && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || b[i] == b'-')
                {
                    i += 1;
                }
                toks.push(Token { kind: Tok::Word(&src[start..i]), at: start });
            }
            _ => return Err(IslError::Lex { at: i }),
        }
    }
    Ok(toks)
}

/// CEP:WHAT: The recursive-descent-free parser state: token cursor.
/// CEP:WHY: Bracket depth is tracked by an integer, so nesting budget breaches are
/// detected without call-stack recursion (38.6).
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: POD.
/// CEP:EVIDENCE: internal.
struct Cursor<'a> {
    toks: Vec<Token<'a>>,
    pos: usize,
}

/// CEP:WHAT: Parses and validates an ISL document into a TargetDescription.
/// CEP:WHY: THE entry point of the crate.
/// CEP:STATUS: complete
/// CEP:FAILURE: IslError for lexical, syntactic, and every structural violation
/// listed in validate; never panics.
/// CEP:ASSUMES: input is UTF-8.
/// CEP:COST: O(text + lanes).
/// CEP:EVIDENCE: the grammar corpus and the cross-check tests in rv8_backend.
pub fn parse(src: &str) -> Result<TargetDescription, IslError> {
    if src.len() > MAX_ISL_BYTES {
        return Err(IslError::Lex { at: MAX_ISL_BYTES });
    }
    let toks = lex(src)?;
    let mut cur = Cursor { toks, pos: 0 };
    parse_document(&mut cur)
}

/// CEP:WHAT: document := 'target' STRING '{' class* abi* frame* '}'.
/// CEP:WHY: Grammar root.
/// CEP:STATUS: complete
/// CEP:FAILURE: IslError::Syntax.
/// CEP:ASSUMES: none
/// CEP:COST: O(tokens).
/// CEP:EVIDENCE: grammar corpus.
fn parse_document(cur: &mut Cursor<'_>) -> Result<TargetDescription, IslError> {
    expect_word(cur, "target")?;
    let name = expect_str(cur)?;
    expect_lbrace(cur)?;
    let mut classes: Vec<Class> = Vec::new();
    let mut return_reg: Option<String> = None;
    let mut args: Vec<String> = Vec::new();
    let mut alignment: Option<u32> = None;
    let mut red_zone_leaf = false;
    let mut open_braces = 1usize; // the document brace is open
    loop {
        match peek_word(cur) {
            Some("class") => {
                open_braces += 1;
                if open_braces > MAX_ISL_DEPTH {
                    return Err(IslError::Syntax { at: cur.pos, want: "shallower nesting" });
                }
                let c = parse_class(cur)?;
                open_braces -= 1;
                if classes.iter().any(|x| x.name == c.name) {
                    return Err(IslError::Invalid {
                        reason: format!("duplicate class {}", c.name),
                    });
                }
                classes.push(c);
            }
            Some("abi") => {
                cur.pos += 1;
                // One clause may carry both sub-keywords: `abi return rax args rdi ..`.
                loop {
                    match next_word(cur)? {
                        "return" => {
                            let r = next_word(cur)?;
                            if return_reg.is_some() {
                                return Err(IslError::Invalid {
                                    reason: "duplicate abi return".into(),
                                });
                            }
                            return_reg = Some(r.to_string());
                        }
                        "args" => {
                            loop {
                                match peek_word(cur) {
                                    Some(w) if is_lane_name(w, cur) => {
                                        args.push(next_word(cur)?.to_string());
                                    }
                                    _ => break,
                                }
                            }
                            if args.is_empty() {
                                return Err(IslError::Invalid {
                                    reason: "abi args needs at least one register".into(),
                                });
                            }
                        }
                        other => {
                            return Err(IslError::Syntax {
                                at: cur.toks[cur.pos.saturating_sub(1)].at,
                                want: other2(other),
                            });
                        }
                    }
                    if !matches!(peek_word(cur), Some("return") | Some("args")) {
                        break;
                    }
                }
            }
            Some("frame") => {
                cur.pos += 1;
                // One clause may carry both sub-keywords:
                // `frame alignment 16 red-zone-leaf`.
                loop {
                    match next_word(cur)? {
                        "alignment" => {
                            let n = expect_num(cur)?;
                            if !(16.0..=1_048_576.0).contains(&n) || n.fract() != 0.0 {
                                return Err(IslError::Invalid {
                                    reason: format!("alignment {n} out of range"),
                                });
                            }
                            alignment = Some(n as u32);
                        }
                        "red-zone-leaf" => red_zone_leaf = true,
                        other => {
                            return Err(IslError::Syntax {
                                at: cur.toks[cur.pos.saturating_sub(1)].at,
                                want: other2(other),
                            });
                        }
                    }
                    if !matches!(peek_word(cur), Some("alignment") | Some("red-zone-leaf")) {
                        break;
                    }
                }
            }
            _ => break,
        }
    }
    expect_rbrace(cur)?;
    let name = name.to_string();
    let return_reg = return_reg.ok_or(IslError::Invalid {
        reason: "missing abi return".into(),
    })?;
    let alignment = alignment.ok_or(IslError::Invalid {
        reason: "missing frame alignment".into(),
    })?;
    let desc = TargetDescription {
        name,
        classes,
        return_reg,
        args,
        frame: Frame { alignment, red_zone_leaf },
    };
    validate(&desc)?;
    Ok(desc)
}

/// CEP:WHAT: Maps a word to the static expectation for syntax errors.
/// CEP:WHY: Better diagnostics without carrying String in Syntax.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: internal.
fn other2(w: &str) -> &'static str {
    match w {
        "return" => "return",
        "args" => "args",
        "alignment" => "alignment",
        "red-zone-leaf" => "red-zone-leaf",
        _ => "abi or frame clause",
    }
}

/// CEP:WHAT: class := 'class' WORD '{' ('move_cost'|'spill_cost'|'lane')* '}'.
/// CEP:WHY: Grammar for one register class.
/// CEP:STATUS: complete
/// CEP:FAILURE: IslError::Syntax and duplicate-lane validation.
/// CEP:ASSUMES: none
/// CEP:COST: O(class tokens).
/// CEP:EVIDENCE: grammar corpus.
fn parse_class(cur: &mut Cursor<'_>) -> Result<Class, IslError> {
    cur.pos += 1; // 'class'
    let name = next_word(cur)?.to_string();
    expect_lbrace(cur)?;
    let mut move_cost: Option<f64> = None;
    let mut spill_cost: Option<f64> = None;
    let mut lanes: Vec<Lane> = Vec::new();
    loop {
        match peek_word(cur) {
            Some("move_cost") => {
                cur.pos += 1;
                let n = expect_num(cur)?;
                if n <= 0.0 {
                    return Err(IslError::Invalid {
                        reason: format!("move_cost {n} must be positive"),
                    });
                }
                move_cost = Some(n);
            }
            Some("spill_cost") => {
                cur.pos += 1;
                let n = expect_num(cur)?;
                if n <= 0.0 {
                    return Err(IslError::Invalid {
                        reason: format!("spill_cost {n} must be positive"),
                    });
                }
                spill_cost = Some(n);
            }
            Some("lane") => {
                cur.pos += 1;
                let lname = next_word(cur)?.to_string();
                expect_word(cur, "enc")?;
                let enc = expect_num(cur)?;
                if enc < 0.0 || enc > 65535.0 || enc.fract() != 0.0 {
                    return Err(IslError::Invalid {
                        reason: format!("enc {enc} out of range"),
                    });
                }
                let mut lane = Lane {
                    name: lname,
                    enc: enc as u16,
                    callee_saved: false,
                    is_return: false,
                    args: Vec::new(),
                    div_clobber: false,
                };
                loop {
                    match peek_word(cur) {
                        Some("caller-saved") => {
                            cur.pos += 1;
                        }
                        Some("callee-saved") => {
                            cur.pos += 1;
                            lane.callee_saved = true;
                        }
                        Some("return") => {
                            cur.pos += 1;
                            lane.is_return = true;
                        }
                        Some("div-clobber") => {
                            cur.pos += 1;
                            lane.div_clobber = true;
                        }
                        Some("arg") => {
                            cur.pos += 1;
                            let k = expect_num(cur)?;
                            if k < 0.0 || k > 5.0 || k.fract() != 0.0 {
                                return Err(IslError::Invalid {
                                    reason: format!("arg position {k} out of range"),
                                });
                            }
                            let kb = k as u8;
                            if !lane.args.contains(&kb) {
                                lane.args.push(kb);
                            }
                        }
                        _ => break,
                    }
                }
                if lanes.iter().any(|l| l.name == lane.name) {
                    return Err(IslError::Invalid {
                        reason: format!("duplicate lane {}", lane.name),
                    });
                }
                lanes.push(lane);
            }
            _ => break,
        }
    }
    expect_rbrace(cur)?;
    Ok(Class {
        name,
        lanes,
        move_cost: move_cost.ok_or(IslError::Invalid {
            reason: "class missing move_cost".into(),
        })?,
        spill_cost: spill_cost.ok_or(IslError::Invalid {
            reason: "class missing spill_cost".into(),
        })?,
    })
}

/// CEP:WHAT: Structural validation of a parsed description.
/// CEP:WHY: Fail-closed target facts: every invariant a consumer relies on is checked
/// here once, not re-derived at each call site.
/// CEP:STATUS: complete
/// CEP:FAILURE: IslError::Invalid naming the violated invariant.
/// CEP:ASSUMES: none
/// CEP:COST: O(lanes).
/// CEP:EVIDENCE: rejection corpus in this module.
fn validate(d: &TargetDescription) -> Result<(), IslError> {
    if d.classes.is_empty() {
        return Err(IslError::Invalid { reason: "no classes".into() });
    }
    for c in &d.classes {
        if c.lanes.is_empty() {
            return Err(IslError::Invalid {
                reason: format!("class {} has no lanes", c.name),
            });
        }
        if c.lanes.len() > 64 {
            return Err(IslError::Invalid {
                reason: format!("class {} exceeds 64 lanes", c.name),
            });
        }
        let mut prev_enc: Option<u16> = None;
        for l in &c.lanes {
            if let Some(p) = prev_enc {
                if l.enc <= p {
                    return Err(IslError::Invalid {
                        reason: format!(
                            "lane {} enc {} breaks the strictly increasing machine-encoding                              order of class {}",
                            l.name, l.enc, c.name
                        ),
                    });
                }
            }
            prev_enc = Some(l.enc);
            if (l.is_return || !l.args.is_empty()) && c.name != d.classes[0].name {
                return Err(IslError::Invalid {
                    reason: "ABI roles are only valid in the first class".into(),
                });
            }
        }
    }
    let gpr = &d.classes[0];
    let find = |name: &str| gpr.lanes.iter().find(|l| l.name == name);
    find(&d.return_reg).ok_or(IslError::Invalid {
        reason: format!("abi return {} not a lane of {}", d.return_reg, gpr.name),
    })?;
    if d.args.len() > 6 {
        return Err(IslError::Invalid {
            reason: format!("{} ABI arguments exceed the six-register maximum", d.args.len()),
        });
    }
    for (k, a) in d.args.iter().enumerate() {
        let lane = find(a).ok_or(IslError::Invalid {
            reason: format!("abi arg {} not a lane of {}", a, gpr.name),
        })?;
        if !lane.args.contains(&(k as u8)) {
            return Err(IslError::Invalid {
                reason: format!("abi arg {} lacks role arg {k}", a),
            });
        }
    }
    let ret_lane = find(&d.return_reg).expect("checked");
    if !ret_lane.is_return {
        return Err(IslError::Invalid {
            reason: format!("abi return {} lacks the return role", d.return_reg),
        });
    }
    if !d.frame.alignment.is_power_of_two() || d.frame.alignment < 16 {
        return Err(IslError::Invalid {
            reason: "frame alignment must be a power of two of at least 16".into(),
        });
    }
    Ok(())
}

/// CEP:WHAT: True when the word at the cursor is a plausible lane reference.
/// CEP:WHY: abi args consume lane names until the next clause keyword.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: internal.
fn is_lane_name(w: &str, cur: &Cursor<'_>) -> bool {
    !matches!(w, "abi" | "frame" | "class") && {
        // A lane name is any word; the validator cross-checks membership.
        let _ = cur;
        w.chars().next().is_some_and(|c| c.is_ascii_lowercase())
    }
}

/// CEP:WHAT: Peeks the next word token without consuming it.
/// CEP:WHY: Parser dispatch.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: internal.
fn peek_word<'a>(cur: &Cursor<'a>) -> Option<&'a str> {
    match cur.toks.get(cur.pos) {
        Some(Token { kind: Tok::Word(w), .. }) => Some(w),
        _ => None,
    }
}

/// CEP:WHAT: Consumes the next word token.
/// CEP:WHY: Parser primitive.
/// CEP:STATUS: complete
/// CEP:FAILURE: Syntax when the token is not a word.
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: internal.
fn next_word<'a>(cur: &mut Cursor<'a>) -> Result<&'a str, IslError> {
    match cur.toks.get(cur.pos) {
        Some(Token { kind: Tok::Word(w), .. }) => {
            cur.pos += 1;
            Ok(w)
        }
        Some(t) => Err(IslError::Syntax { at: t.at, want: "word" }),
        None => Err(IslError::Syntax { at: usize::MAX, want: "word" }),
    }
}

/// CEP:WHAT: Consumes a specific keyword.
/// CEP:WHY: Parser primitive.
/// CEP:STATUS: complete
/// CEP:FAILURE: Syntax with the keyword as the expectation.
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: internal.
fn expect_word(cur: &mut Cursor<'_>, want: &'static str) -> Result<(), IslError> {
    match cur.toks.get(cur.pos) {
        Some(Token { kind: Tok::Word(w), at }) if *w == want => {
            cur.pos += 1;
            Ok(())
        }
        Some(Token { at, .. }) => Err(IslError::Syntax { at: *at, want }),
        None => Err(IslError::Syntax { at: usize::MAX, want }),
    }
}

/// CEP:WHAT: Consumes a quoted string.
/// CEP:WHY: Parser primitive (target names).
/// CEP:STATUS: complete
/// CEP:FAILURE: Syntax when not a string.
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: internal.
fn expect_str<'a>(cur: &mut Cursor<'a>) -> Result<&'a str, IslError> {
    match cur.toks.get(cur.pos) {
        Some(Token { kind: Tok::Str(s), .. }) => {
            cur.pos += 1;
            Ok(s)
        }
        Some(t) => Err(IslError::Syntax { at: t.at, want: "string" }),
        None => Err(IslError::Syntax { at: usize::MAX, want: "string" }),
    }
}

/// CEP:WHAT: Consumes a number.
/// CEP:WHY: Parser primitive (costs, encodings, alignment).
/// CEP:STATUS: complete
/// CEP:FAILURE: Syntax when not a number.
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: internal.
fn expect_num(cur: &mut Cursor<'_>) -> Result<f64, IslError> {
    match cur.toks.get(cur.pos) {
        Some(Token { kind: Tok::Num(n), .. }) => {
            cur.pos += 1;
            Ok(*n)
        }
        Some(t) => Err(IslError::Syntax { at: t.at, want: "number" }),
        None => Err(IslError::Syntax { at: usize::MAX, want: "number" }),
    }
}

/// CEP:WHAT: Consumes '{'.
/// CEP:WHY: Parser primitive.
/// CEP:STATUS: complete
/// CEP:FAILURE: Syntax.
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: internal.
fn expect_lbrace(cur: &mut Cursor<'_>) -> Result<(), IslError> {
    match cur.toks.get(cur.pos) {
        Some(Token { kind: Tok::LBrace, .. }) => {
            cur.pos += 1;
            Ok(())
        }
        Some(t) => Err(IslError::Syntax { at: t.at, want: "'{'" }),
        None => Err(IslError::Syntax { at: usize::MAX, want: "'{'" }),
    }
}

/// CEP:WHAT: Consumes '}'.
/// CEP:WHY: Parser primitive.
/// CEP:STATUS: complete
/// CEP:FAILURE: Syntax.
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: internal.
fn expect_rbrace(cur: &mut Cursor<'_>) -> Result<(), IslError> {
    match cur.toks.get(cur.pos) {
        Some(Token { kind: Tok::RBrace, .. }) => {
            cur.pos += 1;
            Ok(())
        }
        Some(t) => Err(IslError::Syntax { at: t.at, want: "'}'" }),
        None => Err(IslError::Syntax { at: usize::MAX, want: "'}'" }),
    }
}

/// CEP:WHAT: The embedded x86-64 description and its parsed accessor.
/// CEP:WHY: The production target ships with the crate; consumers parse this text.
/// CEP:STATUS: complete
/// CEP:FAILURE: parse errors surface to the caller.
/// CEP:ASSUMES: the file is valid (cross-checked by tests).
/// CEP:COST: cold; O(text) per parse.
/// CEP:EVIDENCE: x86_64_matches_abi_tables in rv8_backend::desc.
pub const X86_64_ISL: &str = include_str!("../targets/x86-64.isl");

/// CEP:WHAT: The embedded toy target description.
/// CEP:WHY: The shared-core demonstration ships with the crate.
/// CEP:STATUS: complete
/// CEP:FAILURE: parse errors surface to the caller.
/// CEP:ASSUMES: the file is valid (tested).
/// CEP:COST: cold.
/// CEP:EVIDENCE: allocate_with_two_targets_share_the_core.
pub const TOY4_ISL: &str = include_str!("../targets/toy4.isl");

/// CEP:WHAT: Parses the embedded x86-64 description.
/// CEP:WHY: Convenience accessor for the production target.
/// CEP:STATUS: complete
/// CEP:FAILURE: IslError (unreachable for the shipped file; tested).
/// CEP:ASSUMES: none
/// CEP:COST: cold.
/// CEP:EVIDENCE: cross-check tests.
pub fn x86_64() -> Result<TargetDescription, IslError> {
    parse(X86_64_ISL)
}

#[cfg(test)]
pub mod tests;
