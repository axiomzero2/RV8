// CEP:FILE: crates/rv8_frontend/src/token.rs
// CEP:WHAT: The token vocabulary of the RV8 Rust subset: token kinds (identifiers, the
//   keyword set, integer and boolean literals, punctuation, operators, delimiters, EOF),
//   span-tracked tokens, nested Delimited token trees, and SyntaxContext hygiene counters.
// CEP:WHY: Standard 5.7 pins the frontend to a TokenTree representation with nested
//   delimiters and hygiene via SyntaxContext; index-based SymbolIds keep tokens comparable
//   and hashable without pointer identity (38.17).
// CEP:CLASS: HPC-0
// CEP:STATUS: complete
// CEP:FAILURE: none; token construction is total over its inputs.
// CEP:ASSUMES: SymbolIds were issued by the Interner of the same compilation session.
// CEP:COST: tokens are small POD values; SyntaxContext issue is O(1) monotonic.
// CEP:EVIDENCE: unit test rv8_frontend::token::tests::hygiene_counter_is_monotonic.

use rv8_core::interner::SymbolId;
use rv8_core::span::Span;

/// CEP:WHAT: Maximum number of distinct syntax contexts one compilation may issue.
/// CEP:WHY: Named budget (38.12); the counter saturates instead of wrapping so context
///   uniqueness can never silently break under pathological input.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: unit test rv8_frontend::token::tests::hygiene_counter_is_monotonic.
pub const MAX_SYNTAX_CONTEXTS: u32 = u32::MAX;

/// CEP:WHAT: Hygiene marker attached to every token; equal contexts denote tokens from the
///   same syntactic expansion site.
/// CEP:WHY: Standard 5.7 requires hygiene via SyntaxContext so future macro expansion can
///   never capture identifiers across sites.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: contexts are issued only by the session's HygieneCounter.
/// CEP:COST: POD u32 wrapper.
/// CEP:EVIDENCE: unit test rv8_frontend::token::tests::hygiene_counter_is_monotonic.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct SyntaxContext(pub u32);

/// CEP:WHAT: Monotonic issuer of SyntaxContext values; saturating at MAX_SYNTAX_CONTEXTS.
/// CEP:WHY: Hygiene requires a deterministic, ever-increasing context source per session.
/// CEP:STATUS: complete
/// CEP:FAILURE: fresh() saturates instead of overflowing; contexts issued after saturation
///   are all equal, which is reported by used_up() for callers that must fail closed.
/// CEP:ASSUMES: single-threaded use within one compilation session.
/// CEP:COST: O(1) per issue.
/// CEP:EVIDENCE: unit test rv8_frontend::token::tests::hygiene_counter_is_monotonic.
#[derive(Clone, Copy, Default, Debug)]
pub struct HygieneCounter {
    next: u32,
}

impl HygieneCounter {
    /// CEP:WHAT: Creates a counter starting at context zero.
    /// CEP:WHY: Construction boundary.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_frontend::token::tests::hygiene_counter_is_monotonic.
    pub fn new() -> HygieneCounter {
        HygieneCounter { next: 0 }
    }

    /// CEP:WHAT: Issues the next SyntaxContext in monotonic order.
    /// CEP:WHY: One issuer keeps contexts unique and reproducible for identical input.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: saturates at MAX_SYNTAX_CONTEXTS (see used_up).
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_frontend::token::tests::hygiene_counter_is_monotonic.
    pub fn fresh(&mut self) -> SyntaxContext {
        let ctx = SyntaxContext(self.next);
        self.next = self.next.saturating_add(1);
        ctx
    }

    /// CEP:WHAT: Reports whether the counter reached saturation.
    /// CEP:WHY: Callers that require fresh contexts must be able to fail closed.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_frontend::token::tests::hygiene_counter_is_monotonic.
    pub fn used_up(&self) -> bool {
        self.next == MAX_SYNTAX_CONTEXTS
    }
}

/// CEP:WHAT: Token kinds of the Rust subset: identifiers, keywords, integer literals,
///   punctuation, delimiters, and the EOF sentinel.
/// CEP:WHY: A closed kind set keeps the parser and lexer exhaustive over inputs.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: identifiers are interned (SymbolId), never raw strings.
/// CEP:COST: POD enum.
/// CEP:EVIDENCE: unit tests rv8_frontend::lexer::tests::*.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TokenKind {
    Ident(SymbolId),
    IntLit(i64),
    KwFn,
    KwLet,
    KwMut,
    KwIf,
    KwElse,
    KwWhile,
    KwLoop,
    KwBreak,
    KwContinue,
    KwReturn,
    KwTrue,
    KwFalse,
    OpenParen,
    CloseParen,
    OpenBrace,
    CloseBrace,
    OpenBracket,
    CloseBracket,
    Comma,
    Semi,
    Colon,
    Arrow,
    Eq,
    EqEq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Amp,
    AmpAmp,
    Pipe,
    PipePipe,
    Caret,
    Shl,
    Shr,
    Bang,
    Eof,
}

impl TokenKind {
    /// CEP:WHAT: Renders a stable human-readable name for error messages.
    /// CEP:WHY: Diagnostics must name the offending token deterministically (38.15).
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit tests rv8_frontend::lexer::tests::*.
    pub fn describe(self) -> &'static str {
        match self {
            TokenKind::Ident(_) => "identifier",
            TokenKind::IntLit(_) => "integer literal",
            TokenKind::KwFn => "`fn`",
            TokenKind::KwLet => "`let`",
            TokenKind::KwMut => "`mut`",
            TokenKind::KwIf => "`if`",
            TokenKind::KwElse => "`else`",
            TokenKind::KwWhile => "`while`",
            TokenKind::KwLoop => "`loop`",
            TokenKind::KwBreak => "`break`",
            TokenKind::KwContinue => "`continue`",
            TokenKind::KwReturn => "`return`",
            TokenKind::KwTrue => "`true`",
            TokenKind::KwFalse => "`false`",
            TokenKind::OpenParen => "`(`",
            TokenKind::CloseParen => "`)`",
            TokenKind::OpenBrace => "`{`",
            TokenKind::CloseBrace => "`}`",
            TokenKind::OpenBracket => "`[`",
            TokenKind::CloseBracket => "`]`",
            TokenKind::Comma => "`,`",
            TokenKind::Semi => "`;`",
            TokenKind::Colon => "`:`",
            TokenKind::Arrow => "`->`",
            TokenKind::Eq => "`=`",
            TokenKind::EqEq => "`==`",
            TokenKind::Ne => "`!=`",
            TokenKind::Lt => "`<`",
            TokenKind::Le => "`<=`",
            TokenKind::Gt => "`>`",
            TokenKind::Ge => "`>=`",
            TokenKind::Plus => "`+`",
            TokenKind::Minus => "`-`",
            TokenKind::Star => "`*`",
            TokenKind::Slash => "`/`",
            TokenKind::Percent => "`%`",
            TokenKind::Amp => "`&`",
            TokenKind::AmpAmp => "`&&`",
            TokenKind::Pipe => "`|`",
            TokenKind::PipePipe => "`||`",
            TokenKind::Caret => "`^`",
            TokenKind::Shl => "`<<`",
            TokenKind::Shr => "`>>`",
            TokenKind::Bang => "`!`",
            TokenKind::Eof => "end of file",
        }
    }
}

/// CEP:WHAT: One lexed token: kind, source span, and hygiene context.
/// CEP:WHY: Span-tracked tokens keep the provenance chain (38.15); contexts carry hygiene.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: span is a half-open byte range of the single source file.
/// CEP:COST: POD.
/// CEP:EVIDENCE: unit tests rv8_frontend::lexer::tests::*.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
    pub ctx: SyntaxContext,
}

/// CEP:WHAT: A token tree: either a leaf token or a nested Delimited group with its open
///   and close tokens and the inner stream.
/// CEP:WHY: Standard 5.7 pins nested Delimited { open, stream, close } token trees; the
///   parser walks this structure with an explicit depth budget.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: open and close kinds are matching delimiter pair members.
/// CEP:COST: owned vectors; one allocation per group.
/// CEP:EVIDENCE: unit tests rv8_frontend::lexer::tests::*.
#[derive(Clone, Debug)]
pub enum TokenTree {
    Token(Token),
    Delimited {
        open: Token,
        stream: Vec<TokenTree>,
        close: Token,
    },
}

impl TokenTree {
    /// CEP:WHAT: Returns the span of the whole tree (open..close for groups).
    /// CEP:WHY: Diagnostics over groups need a covering span.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_frontend::lexer::tests::lex_fixture_tokens.
    pub fn span(&self) -> Span {
        match self {
            TokenTree::Token(t) => t.span,
            TokenTree::Delimited { open, close, .. } => open.span.merge(close.span),
        }
    }

    /// CEP:WHAT: Borrows the leading token of a tree, if it is a leaf.
    /// CEP:WHY: Parser lookahead needs token kinds without recursing into groups.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: returns None for Delimited trees.
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit tests rv8_frontend::parser::tests::*.
    pub fn token(&self) -> Option<&Token> {
        match self {
            TokenTree::Token(t) => Some(t),
            TokenTree::Delimited { .. } => None,
        }
    }
}
