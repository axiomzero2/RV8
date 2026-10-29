// CEP:FILE: crates/rv8_frontend/src/lexer.rs
// CEP:WHAT: The TokenTree lexer of the RV8 Rust subset: identifiers and keywords, decimal
//   integer literals, `//` line and nested `/* block */` comments, all operator and
//   punctuation tokens, and nested Delimited { open, stream, close } token trees, all with
//   byte-accurate spans and monotonic hygiene contexts.
// CEP:WHY: Standard 5.7 pins the frontend to nested-delimiter token trees; the lexer is
//   the trust boundary for raw bytes, so it must reject every malformed input class with a
//   spanned error and never panic (38.6, 38.13).
// CEP:CLASS: HPC-0
// CEP:STATUS: complete
// CEP:FAILURE: returns FrontendError::Lex for unexpected bytes (including non-ASCII),
//   unterminated or mismatched or stray delimiters, EOF-truncated block comments, integer
//   overflow, and literal suffixes; the API is total over any byte input.
// CEP:ASSUMES: input is valid UTF-8 (&str); identifiers are ASCII-only in this subset.
// CEP:COST: O(N) single pass over bytes; one Vec allocation per delimiter group.
// CEP:EVIDENCE: unit tests rv8_frontend::lexer::tests::*.
// CEP:SECURITY: fully iterative (explicit frame stack), so arbitrary nesting depth in the
//   input costs linear memory and zero recursion (38.13).

use crate::error::{FrontendError, LexErrorKind};
use crate::token::{HygieneCounter, Token, TokenKind, TokenTree};
use rv8_core::interner::Interner;
use rv8_core::span::{FileId, Span};

/// CEP:WHAT: Lexer state: byte cursor, open-delimiter stack, per-group child streams,
///   hygiene counter, and the session interner.
/// CEP:WHY: One owner of mutable state keeps the scan loop total and borrow-clean.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: frames entry 0 is the root stream and always exists; opens.len() equals
///   frames.len() - 1.
/// CEP:COST: O(N).
/// CEP:EVIDENCE: unit tests rv8_frontend::lexer::tests::*.
struct Lexer<'i> {
    bytes: &'i [u8],
    file: FileId,
    pos: usize,
    hygiene: HygieneCounter,
    interner: &'i mut Interner,
    opens: Vec<Token>,
    frames: Vec<Vec<TokenTree>>,
}

impl<'i> Lexer<'i> {
    /// CEP:WHAT: Builds a span for a byte range, clamped to the source length.
    /// CEP:WHY: Spans must always be in-range even at EOF-adjacent errors.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: lo <= hi.
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_frontend::lexer::tests::lex_spans_and_contexts.
    fn span(&self, lo: usize, hi: usize) -> Span {
        let end = hi.min(self.bytes.len());
        Span::new(self.file, lo.min(end) as u32, end as u32)
    }

    /// CEP:WHAT: Records one token into the innermost open group.
    /// CEP:WHY: Single emission path keeps spans and contexts uniform.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: the root frame always exists.
    /// CEP:COST: amortized O(1).
    /// CEP:EVIDENCE: unit test rv8_frontend::lexer::tests::lex_fixture_tokens.
    fn push_token(&mut self, kind: TokenKind, lo: usize, hi: usize) {
        let token = Token { kind, span: self.span(lo, hi), ctx: self.hygiene.fresh() };
        if let Some(top) = self.frames.last_mut() {
            top.push(TokenTree::Token(token));
        }
    }

    /// CEP:WHAT: Interns an identifier spelling and returns its kind.
    /// CEP:WHY: Keywords are classified here so the parser matches kinds only.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: text is ASCII (checked by the caller).
    /// CEP:COST: O(len) hash on first sight.
    /// CEP:EVIDENCE: unit test rv8_frontend::lexer::tests::lex_keywords_and_idents.
    fn intern_word(&mut self, text: &str) -> TokenKind {
        match text {
            "fn" => TokenKind::KwFn,
            "let" => TokenKind::KwLet,
            "mut" => TokenKind::KwMut,
            "if" => TokenKind::KwIf,
            "else" => TokenKind::KwElse,
            "while" => TokenKind::KwWhile,
            "loop" => TokenKind::KwLoop,
            "break" => TokenKind::KwBreak,
            "continue" => TokenKind::KwContinue,
            "return" => TokenKind::KwReturn,
            "true" => TokenKind::KwTrue,
            "false" => TokenKind::KwFalse,
            _ => TokenKind::Ident(self.interner.get_or_intern(text)),
        }
    }

    /// CEP:WHAT: Scans an identifier or keyword starting at `lo`.
    /// CEP:WHY: Shared word path for names and the keyword set.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none; only called on a verified ASCII-lead byte.
    /// CEP:ASSUMES: bytes at lo is an ASCII letter or underscore.
    /// CEP:COST: O(len).
    /// CEP:EVIDENCE: unit test rv8_frontend::lexer::tests::lex_keywords_and_idents.
    fn scan_word(&mut self, lo: usize) {
        let mut hi = lo + 1;
        while self.bytes.get(hi).copied().is_some_and(is_ident_continue) {
            hi += 1;
        }
        if let Ok(text) = std::str::from_utf8(&self.bytes[lo..hi]) {
            let kind = self.intern_word(text);
            self.push_token(kind, lo, hi);
        }
        // ASCII-only scanning makes this range valid UTF-8 by construction; a failed
        // conversion would leave no token rather than widen into a panic.
        self.pos = hi;
    }

    /// CEP:WHAT: Scans a decimal integer literal; rejects suffixes and overflow.
    /// CEP:WHY: HIR constants are i64; anything else must be rejected with a span.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: IntLiteralOverflow above i64::MAX; LiteralSuffixUnsupported when an
    ///   identifier character immediately follows the digits.
    /// CEP:ASSUMES: bytes at lo is an ASCII digit.
    /// CEP:COST: O(len).
    /// CEP:EVIDENCE: unit test rv8_frontend::lexer::tests::lex_rejects_bad_bytes_and_literals.
    fn scan_number(&mut self, lo: usize) -> Result<(), FrontendError> {
        let mut value: u64 = 0;
        let mut hi = lo;
        while self.bytes.get(hi).copied().is_some_and(|b| b.is_ascii_digit()) {
            let digit = (self.bytes[hi] - b'0') as u64;
            value = match value.checked_mul(10).and_then(|v| v.checked_add(digit)) {
                Some(v) => v,
                None => {
                    return Err(FrontendError::Lex {
                        kind: LexErrorKind::IntLiteralOverflow,
                        span: self.span(lo, hi + 1),
                    });
                }
            };
            hi += 1;
        }
        if self.bytes.get(hi).copied().is_some_and(is_ident_continue) {
            return Err(FrontendError::Lex {
                kind: LexErrorKind::LiteralSuffixUnsupported,
                span: self.span(lo, hi + 1),
            });
        }
        if value > i64::MAX as u64 {
            return Err(FrontendError::Lex {
                kind: LexErrorKind::IntLiteralOverflow,
                span: self.span(lo, hi),
            });
        }
        self.push_token(TokenKind::IntLit(value as i64), lo, hi);
        self.pos = hi;
        Ok(())
    }

    /// CEP:WHAT: Scans one line comment to end of line or EOF.
    /// CEP:WHY: Comments are trivia; they must consume arbitrary bytes safely.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: bytes[pos..] starts with `//`.
    /// CEP:COST: O(line length).
    /// CEP:EVIDENCE: unit test rv8_frontend::lexer::tests::lex_comments_and_trivia.
    fn scan_line_comment(&mut self) {
        while self.bytes.get(self.pos).copied().is_some_and(|b| b != b'\n') {
            self.pos += 1;
        }
    }

    /// CEP:WHAT: Scans a possibly nested block comment.
    /// CEP:WHY: Rust-subset block comments nest; EOF inside one is a rejected class.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: UnterminatedBlockComment with a span covering the comment start to EOF.
    /// CEP:ASSUMES: bytes[pos..] starts with `/*`.
    /// CEP:COST: O(comment length).
    /// CEP:EVIDENCE: unit test rv8_frontend::lexer::tests::lex_rejects_eof_in_block_comment.
    fn scan_block_comment(&mut self, start: usize) -> Result<(), FrontendError> {
        let mut depth: u32 = 1;
        self.pos += 2;
        while depth > 0 {
            if self.pos + 1 >= self.bytes.len() {
                self.pos = self.bytes.len();
                return Err(FrontendError::Lex {
                    kind: LexErrorKind::UnterminatedBlockComment,
                    span: self.span(start, self.bytes.len()),
                });
            }
            let two = match (self.bytes.get(self.pos), self.bytes.get(self.pos + 1)) {
                (Some(&a), Some(&b)) => [a, b],
                // Both endpoints were bounds-checked before the loop body.
                _ => {
                    self.pos = self.bytes.len();
                    return Err(FrontendError::Lex {
                        kind: LexErrorKind::UnterminatedBlockComment,
                        span: self.span(start, self.bytes.len()),
                    });
                }
            };
            if two == *b"/*" {
                depth = depth.saturating_add(1);
                self.pos += 2;
            } else if two == *b"*/" {
                depth -= 1;
                self.pos += 2;
            } else {
                self.pos += 1;
            }
        }
        Ok(())
    }

    /// CEP:WHAT: Scans an operator or punctuation token with maximal munch.
    /// CEP:WHY: Two-character operators (-> == != <= >= << >> && ||) must win over their
    ///   one-character prefixes.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: UnexpectedChar for any byte outside the subset (including non-ASCII).
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_frontend::lexer::tests::lex_operators_maximal_munch.
    fn scan_operator(&mut self, lo: usize) -> Result<(), FrontendError> {
        let b0 = match self.bytes.get(lo) {
            Some(&b) => b,
            None => return Ok(()),
        };
        let b1 = self.bytes.get(lo + 1).copied();
        let two: Option<TokenKind> = match (b0, b1) {
            (b'-', Some(b'>')) => Some(TokenKind::Arrow),
            (b'=', Some(b'=')) => Some(TokenKind::EqEq),
            (b'!', Some(b'=')) => Some(TokenKind::Ne),
            (b'<', Some(b'=')) => Some(TokenKind::Le),
            (b'>', Some(b'=')) => Some(TokenKind::Ge),
            (b'<', Some(b'<')) => Some(TokenKind::Shl),
            (b'>', Some(b'>')) => Some(TokenKind::Shr),
            (b'&', Some(b'&')) => Some(TokenKind::AmpAmp),
            (b'|', Some(b'|')) => Some(TokenKind::PipePipe),
            _ => None,
        };
        if let Some(kind) = two {
            self.push_token(kind, lo, lo + 2);
            self.pos = lo + 2;
            return Ok(());
        }
        let one = match b0 {
            b'(' => TokenKind::OpenParen,
            b')' => TokenKind::CloseParen,
            b'{' => TokenKind::OpenBrace,
            b'}' => TokenKind::CloseBrace,
            b'[' => TokenKind::OpenBracket,
            b']' => TokenKind::CloseBracket,
            b',' => TokenKind::Comma,
            b';' => TokenKind::Semi,
            b':' => TokenKind::Colon,
            b'=' => TokenKind::Eq,
            b'<' => TokenKind::Lt,
            b'>' => TokenKind::Gt,
            b'+' => TokenKind::Plus,
            b'-' => TokenKind::Minus,
            b'*' => TokenKind::Star,
            b'/' => TokenKind::Slash,
            b'%' => TokenKind::Percent,
            b'&' => TokenKind::Amp,
            b'|' => TokenKind::Pipe,
            b'^' => TokenKind::Caret,
            b'!' => TokenKind::Bang,
            _ => {
                return Err(FrontendError::Lex {
                    kind: LexErrorKind::UnexpectedChar,
                    span: self.span(lo, lo + 1),
                });
            }
        };
        self.push_token(one, lo, lo + 1);
        self.pos = lo + 1;
        Ok(())
    }

    /// CEP:WHAT: Closes the innermost delimiter group into a Delimited tree.
    /// CEP:WHY: The close token must match the open kind, and a close with no open group
    ///   is malformed input.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: StrayClose with no open group; MismatchedDelimiter on kind mismatch.
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1) plus moving the child vector.
    /// CEP:EVIDENCE: unit test rv8_frontend::lexer::tests::lex_rejects_mismatched_delimiters.
    fn close_frame(&mut self, close: Token) -> Result<(), FrontendError> {
        if self.opens.is_empty() {
            return Err(FrontendError::Lex {
                kind: LexErrorKind::StrayClose { found: close_char_of(close.kind) },
                span: close.span,
            });
        }
        let open = self.opens.pop().unwrap_or(close);
        let items = match self.frames.pop() {
            Some(items) => items,
            None => {
                return Err(FrontendError::Internal {
                    context: "lexer frame stack",
                    span: close.span,
                });
            }
        };
        if delimiter_pair(open.kind) != close.kind {
            return Err(FrontendError::Lex {
                kind: LexErrorKind::MismatchedDelimiter {
                    open: open_char_of(open.kind),
                    found: close_char_of(close.kind),
                },
                span: close.span,
            });
        }
        let tree = TokenTree::Delimited { open, stream: items, close };
        if let Some(top) = self.frames.last_mut() {
            top.push(tree);
        }
        Ok(())
    }
}

/// CEP:WHAT: Reports whether a byte may start an identifier.
/// CEP:WHY: Shared predicate keeps the scan loop branch-simple.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: unit test rv8_frontend::lexer::tests::lex_keywords_and_idents.
fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

/// CEP:WHAT: Reports whether a byte may continue an identifier or literal.
/// CEP:WHY: Shared predicate for words, suffix detection, and number scanning.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: unit test rv8_frontend::lexer::tests::lex_keywords_and_idents.
fn is_ident_continue(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// CEP:WHAT: Maps an opening token kind to its character for diagnostics.
/// CEP:WHY: Error messages name the unclosed delimiter.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: unit test rv8_frontend::lexer::tests::lex_rejects_unclosed_delimiter.
fn open_char_of(kind: TokenKind) -> char {
    match kind {
        TokenKind::OpenParen => '(',
        TokenKind::OpenBrace => '{',
        TokenKind::OpenBracket => '[',
        _ => '?',
    }
}

/// CEP:WHAT: Maps a closing token kind to its character for diagnostics.
/// CEP:WHY: Error messages name the mismatched close delimiter.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: unit test rv8_frontend::lexer::tests::lex_rejects_mismatched_delimiters.
fn close_char_of(kind: TokenKind) -> char {
    match kind {
        TokenKind::CloseParen => ')',
        TokenKind::CloseBrace => '}',
        TokenKind::CloseBracket => ']',
        _ => '?',
    }
}

/// CEP:WHAT: Returns the matching close kind for an open kind.
/// CEP:WHY: Delimiter pairing check when closing a group.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: unit test rv8_frontend::lexer::tests::lex_rejects_mismatched_delimiters.
fn delimiter_pair(kind: TokenKind) -> TokenKind {
    match kind {
        TokenKind::OpenParen => TokenKind::CloseParen,
        TokenKind::OpenBrace => TokenKind::CloseBrace,
        TokenKind::OpenBracket => TokenKind::CloseBracket,
        other => other,
    }
}

/// CEP:WHAT: Lexes one source file into a root token-tree stream.
/// CEP:WHY: The single trust boundary between raw bytes and the rest of the frontend.
/// CEP:STATUS: complete
/// CEP:FAILURE: FrontendError::Lex for every malformed-input class named on the module;
///   never panics on any byte input, including non-ASCII and EOF truncation.
/// CEP:ASSUMES: src is valid UTF-8; interner belongs to this compilation session.
/// CEP:COST: O(N) with one Vec per delimiter group.
/// CEP:EVIDENCE: unit tests rv8_frontend::lexer::tests::* and the end-to-end fixture test.
pub fn lex(
    src: &str,
    file: FileId,
    interner: &mut Interner,
) -> Result<Vec<TokenTree>, FrontendError> {
    let mut lx = Lexer {
        bytes: src.as_bytes(),
        file,
        pos: 0,
        hygiene: HygieneCounter::new(),
        interner,
        opens: Vec::new(),
        frames: vec![Vec::new()],
    };
    while let Some(&b) = lx.bytes.get(lx.pos) {
        match b {
            b' ' | b'\t' | b'\n' | b'\r' => lx.pos += 1,
            b'/' if lx.bytes.get(lx.pos + 1) == Some(&b'/') => lx.scan_line_comment(),
            b'/' if lx.bytes.get(lx.pos + 1) == Some(&b'*') => {
                let start = lx.pos;
                lx.scan_block_comment(start)?;
            }
            b if is_ident_start(b) => {
                let lo = lx.pos;
                lx.scan_word(lo);
            }
            b if b.is_ascii_digit() => {
                let lo = lx.pos;
                lx.scan_number(lo)?;
            }
            b'(' | b'{' | b'[' => {
                let lo = lx.pos;
                let kind = match b {
                    b'(' => TokenKind::OpenParen,
                    b'{' => TokenKind::OpenBrace,
                    _ => TokenKind::OpenBracket,
                };
                let token = Token { kind, span: lx.span(lo, lo + 1), ctx: lx.hygiene.fresh() };
                lx.opens.push(token);
                lx.frames.push(Vec::new());
                lx.pos += 1;
            }
            b')' | b'}' | b']' => {
                let lo = lx.pos;
                let kind = match b {
                    b')' => TokenKind::CloseParen,
                    b'}' => TokenKind::CloseBrace,
                    _ => TokenKind::CloseBracket,
                };
                let token = Token { kind, span: lx.span(lo, lo + 1), ctx: lx.hygiene.fresh() };
                lx.close_frame(token)?;
                lx.pos += 1;
            }
            _ => {
                let lo = lx.pos;
                lx.scan_operator(lo)?;
            }
        }
    }
    if let Some(open) = lx.opens.last() {
        return Err(FrontendError::Lex {
            kind: LexErrorKind::UnclosedDelimiter { open: open_char_of(open.kind) },
            span: open.span,
        });
    }
    match lx.frames.pop() {
        Some(root) => Ok(root),
        None => Err(FrontendError::Internal {
            context: "lexer root frame",
            span: Span::new(file, 0, 0),
        }),
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use rv8_core::span::Span;

    /// CEP:WHAT: Counts leaf tokens of a tree stream.
    /// CEP:WHY: Test helper for kind/leaf agreement.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via asserts (test-only).
    /// CEP:ASSUMES: inputs are hand-written and shallow.
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test module.
    fn count_leaves(trees: &[TokenTree]) -> usize {
        let mut n = 0;
        for t in trees {
            match t {
                TokenTree::Token(_) => n += 1,
                TokenTree::Delimited { stream, .. } => n += count_leaves(stream),
            }
        }
        n
    }

    /// CEP:WHAT: Lexes a string with a fresh interner, panicking on error (test-only).
    /// CEP:WHY: Keeps tests short.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test module.
    fn lex_ok(src: &str) -> Vec<TokenTree> {
        let mut ix = Interner::new();
        lex(src, FileId(0), &mut ix).expect("lex should succeed")
    }

    /// CEP:WHAT: Collects leaf token kinds in order.
    /// CEP:WHY: Flat comparison against expected sequences.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via asserts (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test module.
    fn kinds(src: &str) -> Vec<TokenKind> {
        fn walk(trees: &[TokenTree], out: &mut Vec<TokenKind>) {
            for t in trees {
                match t {
                    TokenTree::Token(tok) => out.push(tok.kind),
                    TokenTree::Delimited { stream, .. } => walk(stream, out),
                }
            }
        }
        let mut out = Vec::new();
        walk(&lex_ok(src), &mut out);
        out
    }

    /// CEP:WHAT: Lexes the exact fixture and checks kinds, spans, and grouping.
    /// CEP:WHY: The mandated fixture must lex into two well-formed function items.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn lex_fixture_tokens() {
        let src = "fn add(a: i32, b: i32) -> i32 { a + b }\nfn main() -> i32 { add(6, 7) }";
        let trees = lex_ok(src);
        assert_eq!(trees.len(), 12, "fixture top-level trees");
        let all = kinds(src);
        assert_eq!(all.first(), Some(&TokenKind::KwFn));
        assert!(all.contains(&TokenKind::Arrow));
        assert!(all.contains(&TokenKind::IntLit(6)));
        assert!(all.contains(&TokenKind::IntLit(7)));
        assert_eq!(count_leaves(&trees), all.len());
        let first = trees[0].token().expect("leaf");
        assert_eq!(&src[first.span.lo as usize..first.span.hi as usize], "fn");
    }

    /// CEP:WHAT: Keywords classify; unknown words intern as identifiers.
    /// CEP:WHY: The keyword set is fixed by the brief.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn lex_keywords_and_idents() {
        let ks = kinds("fn let mut if else while loop break continue return true false");
        assert_eq!(ks.len(), 12);
        assert_eq!(ks[0], TokenKind::KwFn);
        assert_eq!(ks[11], TokenKind::KwFalse);
        assert!(kinds("").is_empty());
        let ks2 = kinds("foobar i32 u64");
        assert!(matches!(ks2[0], TokenKind::Ident(_)));
        assert!(matches!(ks2[1], TokenKind::Ident(_)));
        assert!(matches!(ks2[2], TokenKind::Ident(_)));
    }

    /// CEP:WHAT: Nested comments and line comments lex cleanly and produce no tokens.
    /// CEP:WHY: Comment handling is a mandated lexer feature.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn lex_comments_and_trivia() {
        let src = "// line\n/* outer /* nested */ still */ fn f() { /* mid */ } // tail";
        let trees = lex_ok(src);
        assert_eq!(trees.len(), 4, "fn item trees");
        let ks = kinds(src);
        assert_eq!(ks.len(), 2, "comments leave no tokens");
        assert_eq!(ks[0], TokenKind::KwFn);
        assert!(matches!(ks[1], TokenKind::Ident(_)));
    }

    /// CEP:WHAT: Delimiters nest into Delimited trees with matched open/close kinds.
    /// CEP:WHY: The parser relies on the nesting shape.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn lex_nested_groups() {
        let src = "f((1), { 2 })";
        let trees = lex_ok(src);
        assert_eq!(trees.len(), 2, "identifier plus one paren group");
        match &trees[1] {
            TokenTree::Delimited { open, stream, close } => {
                assert_eq!(open.kind, TokenKind::OpenParen);
                assert_eq!(close.kind, TokenKind::CloseParen);
                assert_eq!(stream.len(), 3, "inner group, comma, brace group");
                assert!(matches!(&stream[0], TokenTree::Delimited { .. }));
                assert!(matches!(&stream[2], TokenTree::Delimited { .. }));
            }
            other => panic!("expected delimited group, got {other:?}"),
        }
    }

    /// CEP:WHAT: Two-character operators tokenize with maximal munch.
    /// CEP:WHY: `->` must not split into `-` `>`.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn lex_operators_maximal_munch() {
        let ks = kinds("-> == != <= >= << >> && || a<<1");
        assert_eq!(ks[0], TokenKind::Arrow);
        assert_eq!(ks[1], TokenKind::EqEq);
        assert_eq!(ks[2], TokenKind::Ne);
        assert_eq!(ks[3], TokenKind::Le);
        assert_eq!(ks[4], TokenKind::Ge);
        assert_eq!(ks[5], TokenKind::Shl);
        assert_eq!(ks[6], TokenKind::Shr);
        assert_eq!(ks[7], TokenKind::AmpAmp);
        assert_eq!(ks[8], TokenKind::PipePipe);
        assert_eq!(ks[10], TokenKind::Shl);
    }

    /// CEP:WHAT: An unclosed delimiter is rejected with the opener's span.
    /// CEP:WHY: Negative class: unbalanced delimiters.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn lex_rejects_unclosed_delimiter() {
        let mut ix = Interner::new();
        let err = lex("fn f() -> i32 { (1 + 2", FileId(0), &mut ix).unwrap_err();
        match err {
            FrontendError::Lex { kind: LexErrorKind::UnclosedDelimiter { open }, .. } => {
                assert_eq!(open, '(');
            }
            other => panic!("expected UnclosedDelimiter, got {other:?}"),
        }
    }

    /// CEP:WHAT: A wrong close delimiter and a stray close are both rejected.
    /// CEP:WHY: Negative classes: mismatched and unbalanced delimiters.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn lex_rejects_mismatched_delimiters() {
        let mut ix = Interner::new();
        let err = lex("fn f() { (1 }", FileId(0), &mut ix).unwrap_err();
        match err {
            FrontendError::Lex { kind: LexErrorKind::MismatchedDelimiter { open, found }, .. } => {
                assert_eq!(open, '(');
                assert_eq!(found, '}');
            }
            other => panic!("expected MismatchedDelimiter, got {other:?}"),
        }
        let err2 = lex("fn f() { } )", FileId(0), &mut ix).unwrap_err();
        match err2 {
            FrontendError::Lex { kind: LexErrorKind::StrayClose { found }, .. } => {
                assert_eq!(found, ')');
            }
            other => panic!("expected StrayClose, got {other:?}"),
        }
    }

    /// CEP:WHAT: EOF inside a block comment is rejected.
    /// CEP:WHY: Negative class: EOF-truncated block comment.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn lex_rejects_eof_in_block_comment() {
        let mut ix = Interner::new();
        for src in ["/* abc", "/* a /* b */ c", "/*"] {
            let err = lex(src, FileId(0), &mut ix).unwrap_err();
            match err {
                FrontendError::Lex { kind: LexErrorKind::UnterminatedBlockComment, .. } => {}
                other => panic!("expected UnterminatedBlockComment, got {other:?}"),
            }
        }
    }

    /// CEP:WHAT: Non-ASCII bytes, integer overflow, and literal suffixes are rejected.
    /// CEP:WHY: Negative classes: any byte input must produce Err, never a panic.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn lex_rejects_bad_bytes_and_literals() {
        let mut ix = Interner::new();
        match lex("fn f() { x = 'e'; }", FileId(0), &mut ix) {
            Err(FrontendError::Lex { kind: LexErrorKind::UnexpectedChar, .. }) => {}
            other => panic!("expected UnexpectedChar, got {other:?}"),
        }
        match lex("fn f() { let x = 99999999999999999999999; }", FileId(0), &mut ix) {
            Err(FrontendError::Lex { kind: LexErrorKind::IntLiteralOverflow, .. }) => {}
            other => panic!("expected IntLiteralOverflow, got {other:?}"),
        }
        match lex("fn f() { let x = 5u32; }", FileId(0), &mut ix) {
            Err(FrontendError::Lex { kind: LexErrorKind::LiteralSuffixUnsupported, .. }) => {}
            other => panic!("expected LiteralSuffixUnsupported, got {other:?}"),
        }
    }

    /// CEP:WHAT: Token spans are exact byte ranges and hygiene contexts increase.
    /// CEP:WHY: Provenance (38.15) and hygiene (5.7) contracts.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn lex_spans_and_contexts() {
        let src = "fn  x0";
        let trees = lex_ok(src);
        let t0 = trees[0].token().expect("leaf");
        assert_eq!(t0.span, Span::new(FileId(0), 0, 2));
        let t1 = trees[1].token().expect("leaf");
        assert_eq!(t1.span, Span::new(FileId(0), 4, 6));
        assert!(t1.ctx.0 > t0.ctx.0, "contexts are monotonic");
    }
}
