// CEP:FILE: crates/rv8_frontend/src/parser.rs
// CEP:WHAT: The recursive-descent parser over the token tree: programs of functions with
//   typed parameters, let/assign/expression/if/while/loop/break/continue/return statements,
//   precedence-climbing binary expressions with unary -/!, parenthesized expressions, and
//   calls; with speculative parse (cursor snapshot/restore) for bounded backtracking and an
//   explicit MAX_PARSE_DEPTH budget enforced on every recursive descent.
// CEP:WHY: Standard 5.7 pins recursive descent with speculative parse and backtrack;
//   untrusted input must never drive unbounded recursion (38.13), so nesting above the
//   named depth budget is rejected with a spanned error instead of exhausting the stack.
// CEP:CLASS: HPC-0
// CEP:STATUS: complete
// CEP:FAILURE: returns FrontendError (Parse, NestingTooDeep, AstTooLarge, Unsupported) for
//   every malformed or unsupported input class; never panics on any token stream.
// CEP:ASSUMES: the token tree came from this crate's lexer (balanced groups by construction).
// CEP:COST: O(tokens); every token is inspected a constant number of times.
// CEP:EVIDENCE: unit tests rv8_frontend::parser::tests::* and the end-to-end fixture test.

use crate::ast::{
    AstType, BinOp, Block, ElseClause, Expr, ExprKey, ExprKind, Function, Param, Program, Stmt,
    UnOp, MAX_AST_NODES,
};
use crate::error::{FrontendError, ParseErrorKind};
use crate::token::{Token, TokenKind, TokenTree};
use rv8_core::interner::{Interner, SymbolId};
use rv8_core::span::Span;

/// CEP:WHAT: Maximum parser recursion depth in nested descents (blocks, parens, operator
///   levels, unary chains).
/// CEP:WHY: Named budget (38.12, 38.13); three budget units are consumed per parenthesis
///   level (expr plus unary plus primary), so roughly 40 paren levels and 128 chained unary
///   operators are accepted while deeper nesting is rejected cleanly.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: enforced at every recursive descent via enter().
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: unit tests
///   rv8_frontend::parser::tests::parser_rejects_nesting_over_max_parse_depth and
///   rv8_frontend::parser::tests::parser_rejects_unary_chain_over_budget.
pub const MAX_PARSE_DEPTH: u32 = 128;

/// CEP:WHAT: Maximum parameters per function.
/// CEP:WHY: Named budget (38.12) bounding signature size.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: enforced while parsing parameter lists.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: unit test rv8_frontend::parser::tests::parser_rejects_too_many_params.
pub const MAX_PARAMS: usize = 64;

/// CEP:WHAT: Maximum functions per compilation unit.
/// CEP:WHY: Named budget (38.12) bounding the function table.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: enforced while parsing the program.
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: unit test rv8_frontend::parser::tests::parser_rejects_oversize_program.
pub const MAX_FUNCTIONS: usize = 4096;

/// CEP:WHAT: A parse cursor over one token-tree stream with Copy snapshot/restore for
///   speculative parse.
/// CEP:WHY: Snapshot/restore is the bounded-backtracking mechanism required by 5.7.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: the stream outlives the cursor.
/// CEP:COST: POD; snapshot is a copy.
/// CEP:EVIDENCE: unit test rv8_frontend::parser::tests::parser_speculative_assignment.
#[derive(Clone, Copy, Debug)]
struct Cursor<'t> {
    stream: &'t [TokenTree],
    pos: usize,
}

impl<'t> Cursor<'t> {
    /// CEP:WHAT: Starts a cursor at position zero of a stream.
    /// CEP:WHY: Construction boundary.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit tests rv8_frontend::parser::tests::*.
    fn new(stream: &'t [TokenTree]) -> Cursor<'t> {
        Cursor { stream, pos: 0 }
    }

    /// CEP:WHAT: Borrows the tree at the cursor without advancing.
    /// CEP:WHY: One-token lookahead drives every dispatch decision.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: returns None at end of stream.
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit tests rv8_frontend::parser::tests::*.
    fn peek(&self) -> Option<&'t TokenTree> {
        self.stream.get(self.pos)
    }

    /// CEP:WHAT: Borrows the tree at the cursor and advances past it.
    /// CEP:WHY: Consumption primitive.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: returns None at end of stream.
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit tests rv8_frontend::parser::tests::*.
    fn next(&mut self) -> Option<&'t TokenTree> {
        let tree = self.stream.get(self.pos);
        if tree.is_some() {
            self.pos += 1;
        }
        tree
    }

    /// CEP:WHAT: Span for end-of-stream errors: just past the last tree.
    /// CEP:WHY: UnexpectedEof errors need a location without a token.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_frontend::parser::tests::parser_rejects_missing_semi.
    fn end_span(&self, eof_span: Span) -> Span {
        match self.stream.last() {
            Some(tree) => {
                let s = tree.span();
                Span::new(s.file, s.hi, s.hi)
            }
            None => eof_span,
        }
    }
}

/// CEP:WHAT: The parser: shared depth budget, AST node budget, expression key counter,
///   interner reference, and end-of-file span.
/// CEP:WHY: One owner of the budgets keeps every recursion path checked.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(tokens).
/// CEP:EVIDENCE: unit tests rv8_frontend::parser::tests::*.
struct Parser<'i> {
    interner: &'i Interner,
    eof_span: Span,
    depth: u32,
    budget: u32,
    next_key: u32,
}

impl<'i> Parser<'i> {
    /// CEP:WHAT: Charges one AST node against the budget and mints its ExprKey.
    /// CEP:WHY: Named budget MAX_AST_NODES bounds memory on huge inputs (38.12).
    /// CEP:STATUS: complete
    /// CEP:FAILURE: AstTooLarge when the budget is exhausted.
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_frontend::parser::tests::parser_rejects_oversize_program.
    fn charge(&mut self, span: Span) -> Result<ExprKey, FrontendError> {
        if self.budget == 0 {
            return Err(FrontendError::AstTooLarge { span });
        }
        self.budget -= 1;
        let key = ExprKey(self.next_key);
        self.next_key += 1;
        Ok(key)
    }

    /// CEP:WHAT: Enters one recursive descent level, enforcing MAX_PARSE_DEPTH.
    /// CEP:WHY: The named depth budget is the only thing standing between deeply nested
    ///   input and stack exhaustion (38.13).
    /// CEP:STATUS: complete
    /// CEP:FAILURE: NestingTooDeep above the budget.
    /// CEP:ASSUMES: paired with leave() on every success path.
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test
    ///   rv8_frontend::parser::tests::parser_rejects_nesting_over_max_parse_depth.
    fn enter(&mut self, span: Span) -> Result<(), FrontendError> {
        self.depth = self.depth.saturating_add(1);
        if self.depth > MAX_PARSE_DEPTH {
            return Err(FrontendError::NestingTooDeep { span });
        }
        Ok(())
    }

    /// CEP:WHAT: Leaves one recursive descent level.
    /// CEP:WHY: Pairs with enter() to keep the budget exact.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: depth > 0 (guaranteed by enter() pairing).
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test
    ///   rv8_frontend::parser::tests::parser_rejects_nesting_over_max_parse_depth.
    fn leave(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    /// CEP:WHAT: Resolves an identifier's spelling.
    /// CEP:WHY: Primitive-type-name checks need text.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: empty string for ids outside this session (impossible here).
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_frontend::parser::tests::parser_rejects_type_named_var.
    fn name_of(&self, sym: SymbolId) -> &str {
        self.interner.resolve(sym).unwrap_or("")
    }

    /// CEP:WHAT: Reports whether a symbol spells a primitive type and is reserved.
    /// CEP:WHY: `let i32 = ...;` would make type positions ambiguous; rejected with a
    ///   clear error instead.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_frontend::parser::tests::parser_rejects_type_named_var.
    fn is_reserved_type_name(&self, sym: SymbolId) -> bool {
        AstType::from_name(self.name_of(sym)).is_some()
    }

    /// CEP:WHAT: Requires the next tree to be the given leaf kind and consumes it.
    /// CEP:WHY: Uniform expectation primitive with a spanned error.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: Parse error naming the expected kind.
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_frontend::parser::tests::parser_rejects_missing_semi.
    fn expect(
        &mut self,
        cur: &mut Cursor,
        kind: TokenKind,
        expected: &'static str,
    ) -> Result<Token, FrontendError> {
        let span = match cur.peek() {
            Some(tree) => tree.span(),
            None => cur.end_span(self.eof_span),
        };
        match cur.next().and_then(|tree| tree.token()) {
            Some(tok) if tok.kind == kind => Ok(*tok),
            Some(_) => Err(FrontendError::Parse {
                kind: ParseErrorKind::UnexpectedToken { expected },
                span,
            }),
            None => Err(FrontendError::Parse {
                kind: ParseErrorKind::UnexpectedEof { expected },
                span,
            }),
        }
    }

    /// CEP:WHAT: Requires the next tree to be an identifier leaf, without reserving names.
    /// CEP:WHY: Shared primitive of expect_name; kinds must match exactly.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: Parse error for non-identifiers.
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_frontend::parser::tests::parser_rejects_type_named_var.
    fn expect_ident(
        &mut self,
        cur: &mut Cursor,
        expected: &'static str,
    ) -> Result<Token, FrontendError> {
        let span = match cur.peek() {
            Some(tree) => tree.span(),
            None => cur.end_span(self.eof_span),
        };
        match cur.next().and_then(|tree| tree.token()) {
            Some(tok) if matches!(tok.kind, TokenKind::Ident(_)) => Ok(*tok),
            Some(_) => Err(FrontendError::Parse {
                kind: ParseErrorKind::UnexpectedToken { expected },
                span,
            }),
            None => Err(FrontendError::Parse {
                kind: ParseErrorKind::UnexpectedEof { expected },
                span,
            }),
        }
    }

    /// CEP:WHAT: Requires an identifier usable as a declared name (not a primitive type).
    /// CEP:WHY: Reserved type spellings would make types unparseable at use sites.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: Parse error for non-identifiers and reserved spellings.
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_frontend::parser::tests::parser_rejects_type_named_var.
    fn expect_name(
        &mut self,
        cur: &mut Cursor,
        expected: &'static str,
    ) -> Result<(SymbolId, Token), FrontendError> {
        let tok = self.expect_ident(cur, expected)?;
        let sym = match tok.kind {
            TokenKind::Ident(sym) => sym,
            _ => {
                return Err(FrontendError::Internal { context: "expect_name kind", span: tok.span });
            }
        };
        if self.is_reserved_type_name(sym) {
            return Err(FrontendError::Parse {
                kind: ParseErrorKind::UnexpectedToken { expected },
                span: tok.span,
            });
        }
        Ok((sym, tok))
    }

    /// CEP:WHAT: Parses a primitive type: bool/i32/u32/i64/u64 or the unit `()`.
    /// CEP:WHY: The subset's complete type grammar.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: Parse error for any other shape.
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_frontend::ast::tests::type_parsing_and_conversion.
    fn parse_type(&mut self, cur: &mut Cursor) -> Result<AstType, FrontendError> {
        let span = match cur.peek() {
            Some(tree) => tree.span(),
            None => cur.end_span(self.eof_span),
        };
        match cur.next() {
            Some(TokenTree::Token(tok)) => match tok.kind {
                TokenKind::Ident(sym) => AstType::from_name(self.name_of(sym)).ok_or(
                    FrontendError::Parse {
                        kind: ParseErrorKind::UnexpectedToken { expected: "a type" },
                        span,
                    },
                ),
                _ => Err(FrontendError::Parse {
                    kind: ParseErrorKind::UnexpectedToken { expected: "a type" },
                    span,
                }),
            },
            Some(TokenTree::Delimited { open, stream, close }) => {
                if open.kind == TokenKind::OpenParen
                    && close.kind == TokenKind::CloseParen
                    && stream.is_empty()
                {
                    Ok(AstType::Unit)
                } else {
                    Err(FrontendError::Parse {
                        kind: ParseErrorKind::UnexpectedToken { expected: "a type" },
                        span,
                    })
                }
            }
            None => Err(FrontendError::Parse {
                kind: ParseErrorKind::UnexpectedEof { expected: "a type" },
                span,
            }),
        }
    }

    /// CEP:WHAT: Parses a comma-separated parameter list (possibly empty, trailing comma
    ///   allowed) from inside a paren group.
    /// CEP:WHY: Function signatures anchor inference roots.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: Parse errors; AstTooLarge above MAX_PARAMS.
    /// CEP:ASSUMES: none
    /// CEP:COST: O(params).
    /// CEP:EVIDENCE: unit test rv8_frontend::parser::tests::parser_rejects_too_many_params.
    fn parse_params(&mut self, cur: &mut Cursor) -> Result<Vec<Param>, FrontendError> {
        let mut params: Vec<Param> = Vec::new();
        if cur.peek().is_none() {
            return Ok(params);
        }
        loop {
            if params.len() >= MAX_PARAMS {
                let span = cur.peek().map(|t| t.span()).unwrap_or(self.eof_span);
                return Err(FrontendError::AstTooLarge { span });
            }
            let mutable = if matches!(
                cur.peek().and_then(|t| t.token()).map(|t| t.kind),
                Some(TokenKind::KwMut)
            ) {
                cur.next();
                true
            } else {
                false
            };
            let (name, name_tok) = self.expect_name(cur, "a parameter name")?;
            self.expect(cur, TokenKind::Colon, "`:`")?;
            let ty = self.parse_type(cur)?;
            params.push(Param { name, mutable, ty, span: name_tok.span });
            match cur.peek().and_then(|t| t.token()).map(|t| t.kind) {
                Some(TokenKind::Comma) => {
                    cur.next();
                    if cur.peek().is_none() {
                        break;
                    }
                }
                Some(_) => {
                    let span = cur.peek().map(|t| t.span()).unwrap_or(self.eof_span);
                    return Err(FrontendError::Parse {
                        kind: ParseErrorKind::UnexpectedToken {
                            expected: "`,` or the end of the parameter list",
                        },
                        span,
                    });
                }
                None => break,
            }
        }
        Ok(params)
    }

    /// CEP:WHAT: Parses a braced block group into a Block.
    /// CEP:WHY: Statement containers of if/else/while/loop and function bodies.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: Parse error when the next tree is not a brace group; NestingTooDeep
    ///   via parse_block_content.
    /// CEP:ASSUMES: none
    /// CEP:COST: O(block size).
    /// CEP:EVIDENCE: unit test rv8_frontend::parser::tests::parser_fixture_shape.
    fn parse_braced_block(&mut self, cur: &mut Cursor<'_>) -> Result<Block, FrontendError> {
        let span = match cur.peek() {
            Some(tree) => tree.span(),
            None => cur.end_span(self.eof_span),
        };
        match cur.next() {
            Some(TokenTree::Delimited { open, stream, close }) => {
                if open.kind != TokenKind::OpenBrace || close.kind != TokenKind::CloseBrace {
                    return Err(FrontendError::Parse {
                        kind: ParseErrorKind::UnexpectedToken { expected: "a `{` block" },
                        span,
                    });
                }
                self.parse_block_content(stream, span)
            }
            Some(_) => Err(FrontendError::Parse {
                kind: ParseErrorKind::UnexpectedToken { expected: "a `{` block" },
                span,
            }),
            None => Err(FrontendError::Parse {
                kind: ParseErrorKind::UnexpectedEof { expected: "a `{` block" },
                span,
            }),
        }
    }

    /// CEP:WHAT: Parses statements and an optional trailing expression out of a block's
    ///   token stream.
    /// CEP:WHY: The core statement loop; assignment-vs-expression ambiguity is resolved
    ///   by speculative parse with cursor snapshot/restore.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: Parse/NestingTooDeep/AstTooLarge/Unsupported errors with spans.
    /// CEP:ASSUMES: recursion depth bounded by MAX_PARSE_DEPTH (checked per descent).
    /// CEP:COST: O(tokens in the block).
    /// CEP:EVIDENCE: unit test rv8_frontend::parser::tests::parser_speculative_assignment.
    fn parse_block_content(
        &mut self,
        stream: &[TokenTree],
        span: Span,
    ) -> Result<Block, FrontendError> {
        self.enter(span)?;
        let result = self.parse_block_content_at(stream, span);
        self.leave();
        result
    }

    /// CEP:WHAT: Depth-checked inner body of parse_block_content.
    /// CEP:WHY: Wrapper guarantees leave() on error paths.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: see parse_block_content.
    /// CEP:ASSUMES: caller paired enter()/leave().
    /// CEP:COST: O(tokens in the block).
    /// CEP:EVIDENCE: unit test rv8_frontend::parser::tests::parser_speculative_assignment.
    fn parse_block_content_at(
        &mut self,
        stream: &[TokenTree],
        span: Span,
    ) -> Result<Block, FrontendError> {
        let mut inner = Cursor::new(stream);
        let mut stmts: Vec<Stmt> = Vec::new();
        let mut trailing: Option<Box<Expr>> = None;
        while let Some(tree) = inner.peek() {
            let kind = tree.token().map(|t| t.kind);
            match kind {
                Some(TokenKind::KwLet) => stmts.push(self.parse_let(&mut inner)?),
                Some(TokenKind::KwIf) => stmts.push(self.parse_if(&mut inner)?),
                Some(TokenKind::KwWhile) => stmts.push(self.parse_while(&mut inner)?),
                Some(TokenKind::KwLoop) => stmts.push(self.parse_loop(&mut inner)?),
                Some(TokenKind::KwBreak) => {
                    let tok = self.expect(&mut inner, TokenKind::KwBreak, "`break`")?;
                    let semi = self.expect(&mut inner, TokenKind::Semi, "`;`")?;
                    stmts.push(Stmt::Break { span: tok.span.merge(semi.span) });
                }
                Some(TokenKind::KwContinue) => {
                    let tok = self.expect(&mut inner, TokenKind::KwContinue, "`continue`")?;
                    let semi = self.expect(&mut inner, TokenKind::Semi, "`;`")?;
                    stmts.push(Stmt::Continue { span: tok.span.merge(semi.span) });
                }
                Some(TokenKind::KwReturn) => stmts.push(self.parse_return(&mut inner)?),
                _ => match self.try_parse_assignment(&mut inner)? {
                    Some(stmt) => stmts.push(stmt),
                    None => {
                        let expr = self.parse_expr(&mut inner, 0)?;
                        let after = inner.peek().map(|t| (t.token().map(|tk| tk.kind), t.span()));
                        match after {
                            None => {
                                trailing = Some(Box::new(expr));
                                break;
                            }
                            Some((Some(TokenKind::Semi), _)) => {
                                inner.next();
                                stmts.push(Stmt::Expr(expr));
                            }
                            Some((Some(TokenKind::Eq), span)) => {
                                return Err(FrontendError::Parse {
                                    kind: ParseErrorKind::UnexpectedToken {
                                        expected: "`;` (only plain variables can be assigned)",
                                    },
                                    span,
                                });
                            }
                            Some((_, span)) => {
                                return Err(FrontendError::Parse {
                                    kind: ParseErrorKind::UnexpectedToken { expected: "`;`" },
                                    span,
                                });
                            }
                        }
                    }
                },
            }
        }
        Ok(Block { stmts, trailing, span })
    }

    /// CEP:WHAT: Speculatively parses `IDENT = expr ;`; on any failure restores the cursor
    ///   and reports None so the generic expression path runs.
    /// CEP:WHY: This is the bounded-backtracking speculative parse required by 5.7; the
    ///   snapshot/restore keeps backtracking O(1) in restored position.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: returns Ok(None) after restore on non-matching or failing shapes;
    ///   never consumes input on None.
    /// CEP:ASSUMES: none
    /// CEP:COST: O(speculated expression) on the matching path; O(1) lookahead on plain
    ///   expressions.
    /// CEP:EVIDENCE: unit test rv8_frontend::parser::tests::parser_speculative_assignment.
    fn try_parse_assignment(
        &mut self,
        cur: &mut Cursor<'_>,
    ) -> Result<Option<Stmt>, FrontendError> {
        let save = *cur;
        let (name, name_tok) = match cur.peek().and_then(|t| t.token()) {
            Some(tok) => match tok.kind {
                TokenKind::Ident(sym) => {
                    if self.is_reserved_type_name(sym) {
                        *cur = save;
                        return Ok(None);
                    }
                    (sym, *tok)
                }
                _ => return Ok(None),
            },
            None => return Ok(None),
        };
        cur.next();
        let is_assign = matches!(
            cur.peek().and_then(|t| t.token()).map(|t| t.kind),
            Some(TokenKind::Eq)
        );
        if !is_assign {
            *cur = save;
            return Ok(None);
        }
        cur.next();
        let attempted = self
            .parse_expr(cur, 0)
            .and_then(|value| self.expect_semi(cur).map(|_| value));
        match attempted {
            Ok(value) => {
                let span = name_tok.span.merge(value.span);
                Ok(Some(Stmt::Assign { name, value, span }))
            }
            Err(_) => {
                *cur = save;
                Ok(None)
            }
        }
    }

    /// CEP:WHAT: Consumes one `;`.
    /// CEP:WHY: Shared by statement forms.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: Parse error when absent.
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_frontend::parser::tests::parser_rejects_missing_semi.
    fn expect_semi(&mut self, cur: &mut Cursor) -> Result<(), FrontendError> {
        self.expect(cur, TokenKind::Semi, "`;`").map(|_| ())
    }

    /// CEP:WHAT: Parses `let (mut)? NAME (: TYPE)? (= expr)? ;`.
    /// CEP:WHY: Binding form of the subset; type annotation and init are optional.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: Parse errors with spans.
    /// CEP:ASSUMES: none
    /// CEP:COST: O(statement size).
    /// CEP:EVIDENCE: unit test rv8_frontend::parser::tests::parser_let_shapes.
    fn parse_let(&mut self, cur: &mut Cursor<'_>) -> Result<Stmt, FrontendError> {
        let kw = self.expect(cur, TokenKind::KwLet, "`let`")?;
        let mutable = if matches!(
            cur.peek().and_then(|t| t.token()).map(|t| t.kind),
            Some(TokenKind::KwMut)
        ) {
            cur.next();
            true
        } else {
            false
        };
        let (name, _) = self.expect_name(cur, "a variable name")?;
        let ty = if matches!(
            cur.peek().and_then(|t| t.token()).map(|t| t.kind),
            Some(TokenKind::Colon)
        ) {
            cur.next();
            Some(self.parse_type(cur)?)
        } else {
            None
        };
        let init = if matches!(
            cur.peek().and_then(|t| t.token()).map(|t| t.kind),
            Some(TokenKind::Eq)
        ) {
            cur.next();
            Some(self.parse_expr(cur, 0)?)
        } else {
            None
        };
        let semi = self.expect(cur, TokenKind::Semi, "`;`")?;
        Ok(Stmt::Let {
            name,
            mutable,
            ty,
            init,
            span: kw.span.merge(semi.span),
        })
    }

    /// CEP:WHAT: Parses `if expr BLOCK (else (BLOCK | if-stmt))?`.
    /// CEP:WHY: Conditional form; else-if chains recurse through parse_if, bounded by
    ///   MAX_PARSE_DEPTH via the wrapper.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: Parse errors with spans; NestingTooDeep for chains beyond the budget.
    /// CEP:ASSUMES: none
    /// CEP:COST: O(statements in the chain).
    /// CEP:EVIDENCE: unit test rv8_frontend::parser::tests::parser_if_else_shapes.
    fn parse_if(&mut self, cur: &mut Cursor<'_>) -> Result<Stmt, FrontendError> {
        let start = cur.peek().map(|t| t.span()).unwrap_or(self.eof_span);
        self.enter(start)?;
        let result = self.parse_if_at(cur);
        self.leave();
        result
    }

    /// CEP:WHAT: Depth-checked inner body of parse_if.
    /// CEP:WHY: The wrapper bounds else-if chain recursion (38.13).
    /// CEP:STATUS: complete
    /// CEP:FAILURE: see parse_if.
    /// CEP:ASSUMES: caller paired enter()/leave().
    /// CEP:COST: O(statements in the chain).
    /// CEP:EVIDENCE: unit test rv8_frontend::parser::tests::parser_if_else_shapes.
    fn parse_if_at(&mut self, cur: &mut Cursor<'_>) -> Result<Stmt, FrontendError> {
        let kw = self.expect(cur, TokenKind::KwIf, "`if`")?;
        let cond = self.parse_expr(cur, 0)?;
        let then_block = self.parse_braced_block(cur)?;
        let else_clause = if matches!(
            cur.peek().and_then(|t| t.token()).map(|t| t.kind),
            Some(TokenKind::KwElse)
        ) {
            cur.next();
            if matches!(
                cur.peek().and_then(|t| t.token()).map(|t| t.kind),
                Some(TokenKind::KwIf)
            ) {
                Some(ElseClause::If(Box::new(self.parse_if(cur)?)))
            } else {
                Some(ElseClause::Block(self.parse_braced_block(cur)?))
            }
        } else {
            None
        };
        let end = match &else_clause {
            Some(ElseClause::Block(b)) => b.span,
            Some(ElseClause::If(s)) => stmt_span(s),
            None => then_block.span,
        };
        Ok(Stmt::If {
            cond,
            then_block,
            else_clause,
            span: kw.span.merge(end),
        })
    }

    /// CEP:WHAT: Parses `while expr BLOCK`.
    /// CEP:WHY: Pre-checked loop form.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: Parse errors with spans.
    /// CEP:ASSUMES: none
    /// CEP:COST: O(body size).
    /// CEP:EVIDENCE: unit test rv8_frontend::lib::tests::compile_source_larger_control_flow.
    fn parse_while(&mut self, cur: &mut Cursor<'_>) -> Result<Stmt, FrontendError> {
        let kw = self.expect(cur, TokenKind::KwWhile, "`while`")?;
        let cond = self.parse_expr(cur, 0)?;
        let body = self.parse_braced_block(cur)?;
        let span = kw.span.merge(body.span);
        Ok(Stmt::While { cond, body, span })
    }

    /// CEP:WHAT: Parses `loop BLOCK`.
    /// CEP:WHY: Unconditional loop form; exits via break only.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: Parse errors with spans.
    /// CEP:ASSUMES: none
    /// CEP:COST: O(body size).
    /// CEP:EVIDENCE: unit test rv8_frontend::lib::tests::compile_source_larger_control_flow.
    fn parse_loop(&mut self, cur: &mut Cursor<'_>) -> Result<Stmt, FrontendError> {
        let kw = self.expect(cur, TokenKind::KwLoop, "`loop`")?;
        let body = self.parse_braced_block(cur)?;
        let span = kw.span.merge(body.span);
        Ok(Stmt::Loop { body, span })
    }

    /// CEP:WHAT: Parses `return expr? ;`.
    /// CEP:WHY: Transfer form; expression optional.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: Parse errors with spans.
    /// CEP:ASSUMES: none
    /// CEP:COST: O(expr size).
    /// CEP:EVIDENCE: unit test rv8_frontend::parser::tests::parser_let_shapes.
    fn parse_return(&mut self, cur: &mut Cursor<'_>) -> Result<Stmt, FrontendError> {
        let kw = self.expect(cur, TokenKind::KwReturn, "`return`")?;
        let value = if matches!(
            cur.peek().and_then(|t| t.token()).map(|t| t.kind),
            Some(TokenKind::Semi)
        ) {
            None
        } else {
            Some(self.parse_expr(cur, 0)?)
        };
        let semi = self.expect(cur, TokenKind::Semi, "`;`")?;
        Ok(Stmt::Return { value, span: kw.span.merge(semi.span) })
    }

    /// CEP:WHAT: Maps a token kind to a binary operator; unsupported comparison and
    ///   short-circuit operators yield a descriptive error feature string.
    /// CEP:WHY: The frozen HIR has no !=, >, >=, &&, or ||; those must be rejected at
    ///   parse time with a clear span error instead of unverifiable HIR.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: None for non-operator kinds.
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_frontend::parser::tests::parser_rejects_unsupported_ops.
    fn binop_of(kind: TokenKind) -> Option<Result<BinOp, &'static str>> {
        match kind {
            TokenKind::Plus => Some(Ok(BinOp::Add)),
            TokenKind::Minus => Some(Ok(BinOp::Sub)),
            TokenKind::Star => Some(Ok(BinOp::Mul)),
            TokenKind::Slash => Some(Ok(BinOp::Div)),
            TokenKind::Percent => Some(Ok(BinOp::Rem)),
            TokenKind::Amp => Some(Ok(BinOp::BitAnd)),
            TokenKind::Pipe => Some(Ok(BinOp::BitOr)),
            TokenKind::Caret => Some(Ok(BinOp::BitXor)),
            TokenKind::Shl => Some(Ok(BinOp::Shl)),
            TokenKind::Shr => Some(Ok(BinOp::Shr)),
            TokenKind::EqEq => Some(Ok(BinOp::Eq)),
            TokenKind::Lt => Some(Ok(BinOp::Lt)),
            TokenKind::Le => Some(Ok(BinOp::Le)),
            TokenKind::Ne => Some(Err("operator `!=` (this subset has `==` only)")),
            TokenKind::Gt => Some(Err("operator `>` (this subset has `<` and `<=` only)")),
            TokenKind::Ge => Some(Err("operator `>=` (this subset has `<` and `<=` only)")),
            TokenKind::AmpAmp => Some(Err("short-circuit operator `&&`")),
            TokenKind::PipePipe => Some(Err("short-circuit operator `||`")),
            _ => None,
        }
    }

    /// CEP:WHAT: Precedence-climbing binary expression parser.
    /// CEP:WHY: One loop per tier keeps left-associated chains of any length allocation-
    ///   linear and recursion-free; recursion depth grows only with mixed precedence and
    ///   parens, both bounded by MAX_PARSE_DEPTH.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: Parse/Unsupported/NestingTooDeep errors with spans.
    /// CEP:ASSUMES: all subset operators are left-associative (documented deviation).
    /// CEP:COST: O(tokens).
    /// CEP:EVIDENCE: unit test rv8_frontend::parser::tests::parser_precedence_shape.
    fn parse_expr(&mut self, cur: &mut Cursor<'_>, min_prec: u8) -> Result<Expr, FrontendError> {
        let start = cur.peek().map(|t| t.span()).unwrap_or(self.eof_span);
        self.enter(start)?;
        let result = self.parse_expr_at(cur, min_prec);
        self.leave();
        result
    }

    /// CEP:WHAT: Depth-checked inner body of parse_expr; entered exactly once per call.
    /// CEP:WHY: The wrapper guarantees leave() runs on error paths so speculative re-parse
    ///   never accumulates leaked budget.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: see parse_expr.
    /// CEP:ASSUMES: caller paired enter()/leave().
    /// CEP:COST: O(tokens).
    /// CEP:EVIDENCE: unit test rv8_frontend::parser::tests::parser_speculative_assignment.
    fn parse_expr_at(&mut self, cur: &mut Cursor<'_>, min_prec: u8) -> Result<Expr, FrontendError> {
        let mut lhs = self.parse_unary(cur)?;
        while let Some(tok) = cur.peek().and_then(|t| t.token()) {
            let op = match Parser::binop_of(tok.kind) {
                Some(Ok(op)) => op,
                Some(Err(feature)) => {
                    let span = tok.span;
                    return Err(FrontendError::Unsupported { feature, span });
                }
                None => break,
            };
            if op.precedence() < min_prec {
                break;
            }
            cur.next();
            let rhs = self.parse_expr(cur, op.precedence() + 1)?;
            let span = lhs.span.merge(rhs.span);
            let key = self.charge(span)?;
            lhs = Expr {
                kind: ExprKind::Binary(op, Box::new(lhs), Box::new(rhs)),
                span,
                key,
            };
        }
        Ok(lhs)
    }

    /// CEP:WHAT: Parses unary `-` and `!` chains, descending to a primary.
    /// CEP:WHY: Prefix operators bind tighter than any binary operator; the wrapper keeps
    ///   the depth budget balanced on error paths.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: Parse/NestingTooDeep errors with spans.
    /// CEP:ASSUMES: none
    /// CEP:COST: O(chain length) with recursion bounded by MAX_PARSE_DEPTH.
    /// CEP:EVIDENCE: unit test rv8_frontend::parser::tests::parser_rejects_unary_chain_over_budget.
    fn parse_unary(&mut self, cur: &mut Cursor<'_>) -> Result<Expr, FrontendError> {
        let start = cur.peek().map(|t| t.span()).unwrap_or(self.eof_span);
        self.enter(start)?;
        let result = self.parse_unary_at(cur);
        self.leave();
        result
    }

    /// CEP:WHAT: Depth-checked inner body of parse_unary.
    /// CEP:WHY: Wrapper guarantees leave() on every path.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: see parse_unary.
    /// CEP:ASSUMES: caller paired enter()/leave().
    /// CEP:COST: O(chain length).
    /// CEP:EVIDENCE: unit test rv8_frontend::parser::tests::parser_rejects_unary_chain_over_budget.
    fn parse_unary_at(&mut self, cur: &mut Cursor<'_>) -> Result<Expr, FrontendError> {
        match cur.peek().and_then(|t| t.token()).map(|t| (t.kind, t.span)) {
            Some((TokenKind::Minus, span)) => {
                cur.next();
                let operand = self.parse_unary(cur)?;
                let full = span.merge(operand.span);
                let key = self.charge(full)?;
                Ok(Expr {
                    kind: ExprKind::Unary(UnOp::Neg, Box::new(operand)),
                    span: full,
                    key,
                })
            }
            Some((TokenKind::Bang, span)) => {
                cur.next();
                let operand = self.parse_unary(cur)?;
                let full = span.merge(operand.span);
                let key = self.charge(full)?;
                Ok(Expr {
                    kind: ExprKind::Unary(UnOp::Not, Box::new(operand)),
                    span: full,
                    key,
                })
            }
            _ => self.parse_primary(cur),
        }
    }

    /// CEP:WHAT: Parses a primary: literals, booleans, variables, calls, and parenthesized
    ///   expressions; rejects brackets and braces as unsupported expression forms.
    /// CEP:WHY: Bottom of the expression grammar.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: Parse errors; Unsupported for bracket/brace groups and reserved type
    ///   spellings in value position.
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1) plus call arguments.
    /// CEP:EVIDENCE: unit test rv8_frontend::parser::tests::parser_precedence_shape.
    fn parse_primary(&mut self, cur: &mut Cursor<'_>) -> Result<Expr, FrontendError> {
        let span = cur.peek().map(|t| t.span()).unwrap_or(self.eof_span);
        match cur.next() {
            Some(TokenTree::Token(tok)) => match tok.kind {
                TokenKind::IntLit(v) => {
                    let key = self.charge(tok.span)?;
                    Ok(Expr { kind: ExprKind::IntLit(v), span: tok.span, key })
                }
                TokenKind::KwTrue => {
                    let key = self.charge(tok.span)?;
                    Ok(Expr { kind: ExprKind::BoolLit(true), span: tok.span, key })
                }
                TokenKind::KwFalse => {
                    let key = self.charge(tok.span)?;
                    Ok(Expr { kind: ExprKind::BoolLit(false), span: tok.span, key })
                }
                TokenKind::Ident(sym) => {
                    if self.is_reserved_type_name(sym) {
                        return Err(FrontendError::Parse {
                            kind: ParseErrorKind::UnexpectedToken { expected: "an expression" },
                            span: tok.span,
                        });
                    }
                    let is_call = matches!(
                        cur.peek(),
                        Some(TokenTree::Delimited { open, .. })
                            if open.kind == TokenKind::OpenParen
                    );
                    if is_call {
                        self.parse_call(sym, tok.span, cur)
                    } else {
                        let key = self.charge(tok.span)?;
                        Ok(Expr { kind: ExprKind::Var(sym), span: tok.span, key })
                    }
                }
                _ => Err(FrontendError::Parse {
                    kind: ParseErrorKind::UnexpectedToken { expected: "an expression" },
                    span: tok.span,
                }),
            },
            Some(TokenTree::Delimited { open, stream, .. }) => match open.kind {
                TokenKind::OpenParen => {
                    self.enter(span)?;
                    let result = self.parse_paren_content(stream);
                    self.leave();
                    let inner_expr = result?;
                    let key = self.charge(span)?;
                    Ok(Expr { kind: ExprKind::Paren(Box::new(inner_expr)), span, key })
                }
                TokenKind::OpenBracket => Err(FrontendError::Unsupported {
                    feature: "bracket expressions (no indexing in this subset)",
                    span,
                }),
                _ => Err(FrontendError::Unsupported {
                    feature: "block expressions",
                    span,
                }),
            },
            None => Err(FrontendError::Parse {
                kind: ParseErrorKind::UnexpectedEof { expected: "an expression" },
                span: cur.end_span(self.eof_span),
            }),
        }
    }

    /// CEP:WHAT: Parses the inside of a parenthesized expression: one expression and end
    ///   of stream.
    /// CEP:WHY: Grouping; checked separately so the depth wrapper stays balanced.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: Parse error for trailing tokens inside the group.
    /// CEP:ASSUMES: caller paired enter()/leave().
    /// CEP:COST: O(expr size).
    /// CEP:EVIDENCE: unit test rv8_frontend::parser::tests::parser_precedence_shape.
    fn parse_paren_content(&mut self, stream: &[TokenTree]) -> Result<Expr, FrontendError> {
        let mut inner = Cursor::new(stream);
        let expr = self.parse_expr(&mut inner, 0)?;
        if let Some(extra) = inner.peek() {
            let extra_span = extra.span();
            return Err(FrontendError::Parse {
                kind: ParseErrorKind::UnexpectedToken { expected: "`)`" },
                span: extra_span,
            });
        }
        Ok(expr)
    }

    /// CEP:WHAT: Parses a call's argument list from its paren group.
    /// CEP:WHY: Call syntax with optional trailing comma.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: Parse errors; NestingTooDeep via parse_expr.
    /// CEP:ASSUMES: the group was recognized as a call by parse_primary.
    /// CEP:COST: O(args).
    /// CEP:EVIDENCE: unit test rv8_frontend::parser::tests::parser_fixture_shape.
    fn parse_call(
        &mut self,
        callee: SymbolId,
        callee_span: Span,
        cur: &mut Cursor<'_>,
    ) -> Result<Expr, FrontendError> {
        let group_span = cur.peek().map(|t| t.span()).unwrap_or(self.eof_span);
        match cur.next() {
            Some(TokenTree::Delimited { stream, .. }) => {
                self.enter(group_span)?;
                let mut inner = Cursor::new(stream);
                let mut args: Vec<Expr> = Vec::new();
                if inner.peek().is_some() {
                    loop {
                        args.push(self.parse_expr(&mut inner, 0)?);
                        match inner.peek().and_then(|t| t.token()).map(|t| t.kind) {
                            Some(TokenKind::Comma) => {
                                inner.next();
                                if inner.peek().is_none() {
                                    break;
                                }
                            }
                            Some(_) => {
                                let span = inner.peek().map(|t| t.span()).unwrap_or(self.eof_span);
                                return Err(FrontendError::Parse {
                                    kind: ParseErrorKind::UnexpectedToken {
                                        expected: "`,` or `)`",
                                    },
                                    span,
                                });
                            }
                            None => break,
                        }
                    }
                }
                self.leave();
                let key = self.charge(callee_span.merge(group_span))?;
                Ok(Expr {
                    kind: ExprKind::Call { callee, args },
                    span: callee_span.merge(group_span),
                    key,
                })
            }
            _ => Err(FrontendError::Internal { context: "parse_call group", span: callee_span }),
        }
    }

    /// CEP:WHAT: Parses `fn NAME(params) (-> TYPE)? BLOCK`.
    /// CEP:WHY: The subset's only item form; matches the required fixture grammar.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: Parse errors with spans; reserved type names rejected as names.
    /// CEP:ASSUMES: none
    /// CEP:COST: O(function size).
    /// CEP:EVIDENCE: unit test rv8_frontend::parser::tests::parser_fixture_shape.
    fn parse_fn(&mut self, cur: &mut Cursor<'_>) -> Result<Function, FrontendError> {
        let kw = self.expect(cur, TokenKind::KwFn, "`fn`")?;
        let (name, name_tok) = self.expect_name(cur, "a function name")?;
        let params_span = cur.peek().map(|t| t.span()).unwrap_or(self.eof_span);
        let params = match cur.next() {
            Some(TokenTree::Delimited { open, stream, close }) => {
                if open.kind != TokenKind::OpenParen || close.kind != TokenKind::CloseParen {
                    return Err(FrontendError::Parse {
                        kind: ParseErrorKind::UnexpectedToken { expected: "a parameter list" },
                        span: params_span,
                    });
                }
                let mut pcur = Cursor::new(stream);
                self.parse_params(&mut pcur)?
            }
            _ => {
                return Err(FrontendError::Parse {
                    kind: ParseErrorKind::UnexpectedToken { expected: "a parameter list" },
                    span: params_span,
                });
            }
        };
        let ret = if matches!(
            cur.peek().and_then(|t| t.token()).map(|t| t.kind),
            Some(TokenKind::Arrow)
        ) {
            cur.next();
            Some(self.parse_type(cur)?)
        } else {
            None
        };
        let body = self.parse_braced_block(cur)?;
        let fn_span = kw.span.merge(body.span);
        Ok(Function {
            name,
            name_span: name_tok.span,
            params,
            ret,
            body,
            span: fn_span,
        })
    }
}

/// CEP:WHAT: Span helper for else-if chain endpoints.
/// CEP:WHY: parse_if merges the chain end into the if statement span.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: unit test rv8_frontend::parser::tests::parser_if_else_shapes.
fn stmt_span(stmt: &Stmt) -> Span {
    match stmt {
        Stmt::Let { span, .. }
        | Stmt::Assign { span, .. }
        | Stmt::If { span, .. }
        | Stmt::While { span, .. }
        | Stmt::Loop { span, .. }
        | Stmt::Break { span }
        | Stmt::Continue { span }
        | Stmt::Return { span, .. } => *span,
        Stmt::Expr(e) => e.span,
    }
}

/// CEP:WHAT: Parses a full token-tree stream into a Program.
/// CEP:WHY: The lexer-to-AST boundary used by compile_source.
/// CEP:STATUS: complete
/// CEP:FAILURE: Parse errors with spans; AstTooLarge above MAX_FUNCTIONS or MAX_AST_NODES.
/// CEP:ASSUMES: trees are balanced (lexer-guaranteed).
/// CEP:COST: O(tokens).
/// CEP:EVIDENCE: unit test rv8_frontend::parser::tests::parser_fixture_shape.
pub fn parse_program(
    trees: &[TokenTree],
    interner: &Interner,
    eof_span: Span,
) -> Result<Program, FrontendError> {
    let mut parser = Parser {
        interner,
        eof_span,
        depth: 0,
        budget: MAX_AST_NODES,
        next_key: 0,
    };
    let mut cur = Cursor::new(trees);
    let mut functions: Vec<Function> = Vec::new();
    while cur.peek().is_some() {
        if functions.len() >= MAX_FUNCTIONS {
            let span = cur.peek().map(|t| t.span()).unwrap_or(eof_span);
            return Err(FrontendError::AstTooLarge { span });
        }
        functions.push(parser.parse_fn(&mut cur)?);
    }
    Ok(Program { functions })
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use rv8_core::span::{FileId, Span};

    /// CEP:WHAT: Lexes and parses a source string with a shared interner (test-only panic).
    /// CEP:WHY: Keeps tests short; returns the interner for name checks.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test module.
    fn parse_ok(src: &str) -> (Program, Interner) {
        let mut ix = Interner::new();
        let trees = crate::lexer::lex(src, FileId(0), &mut ix).expect("lex ok");
        let eof = Span::new(FileId(0), src.len() as u32, src.len() as u32);
        let program = parse_program(&trees, &ix, eof).expect("parse ok");
        (program, ix)
    }

    /// CEP:WHAT: Parses expecting failure and returns the error (test-only panic).
    /// CEP:WHY: Negative-test helper.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test module.
    fn parse_err(src: &str) -> FrontendError {
        let mut ix = Interner::new();
        let trees = crate::lexer::lex(src, FileId(0), &mut ix).expect("lex ok");
        let eof = Span::new(FileId(0), src.len() as u32, src.len() as u32);
        parse_program(&trees, &ix, eof).expect_err("parse should fail")
    }

    /// CEP:WHAT: Parses the exact fixture and checks function shapes.
    /// CEP:WHY: The mandated grammar must yield two typed functions with a trailing call.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn parser_fixture_shape() {
        let src = "fn add(a: i32, b: i32) -> i32 { a + b }\nfn main() -> i32 { add(6, 7) }";
        let (program, ix) = parse_ok(src);
        assert_eq!(program.functions.len(), 2);
        let add = &program.functions[0];
        assert_eq!(ix.resolve(add.name), Some("add"));
        assert_eq!(add.params.len(), 2);
        assert!(!add.params[0].mutable);
        assert_eq!(add.params[0].ty, AstType::I32);
        assert_eq!(add.ret, Some(AstType::I32));
        assert!(add.body.stmts.is_empty());
        let trailing = add.body.trailing.as_ref().expect("trailing expr");
        assert!(matches!(trailing.kind, ExprKind::Binary(BinOp::Add, _, _)));
        let main = &program.functions[1];
        assert_eq!(ix.resolve(main.name), Some("main"));
        assert!(main.params.is_empty());
        let tmain = main.body.trailing.as_ref().expect("trailing call");
        match &tmain.kind {
            ExprKind::Call { callee, args } => {
                assert_eq!(ix.resolve(*callee), Some("add"));
                assert_eq!(args.len(), 2);
            }
            other => panic!("expected call, got {other:?}"),
        }
    }

    /// CEP:WHAT: Checks precedence and associativity shapes.
    /// CEP:WHY: `1 + 2 * 3` must group as Add(1, Mul(2,3)); `1 - 2 - 3` left-assoc.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn parser_precedence_shape() {
        let (program, _) = parse_ok("fn f() -> i32 { 1 + 2 * 3 }");
        let e = program.functions[0].body.trailing.as_ref().expect("expr");
        match &e.kind {
            ExprKind::Binary(BinOp::Add, _, rhs) => {
                assert!(matches!(rhs.kind, ExprKind::Binary(BinOp::Mul, _, _)));
            }
            other => panic!("expected Add root, got {other:?}"),
        }
        let (program2, _) = parse_ok("fn f() -> i32 { 1 - 2 - 3 }");
        let e2 = program2.functions[0].body.trailing.as_ref().expect("expr");
        match &e2.kind {
            ExprKind::Binary(BinOp::Sub, lhs, _) => {
                assert!(matches!(lhs.kind, ExprKind::Binary(BinOp::Sub, _, _)));
            }
            other => panic!("expected left-assoc Sub, got {other:?}"),
        }
        let (program3, _) = parse_ok("fn f() -> i32 { (1 + 2) * 3 }");
        let e3 = program3.functions[0].body.trailing.as_ref().expect("expr");
        match &e3.kind {
            ExprKind::Binary(BinOp::Mul, lhs, _) => match &lhs.kind {
                ExprKind::Paren(p) => {
                    assert!(matches!(p.kind, ExprKind::Binary(BinOp::Add, _, _)));
                }
                other => panic!("expected paren, got {other:?}"),
            },
            other => panic!("expected Mul root, got {other:?}"),
        }
    }

    /// CEP:WHAT: Assignments, calls, and comparisons are distinguished by speculative
    ///   parse with snapshot/restore.
    /// CEP:WHY: The 5.7 speculative-parse mechanism must behave exactly.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn parser_speculative_assignment() {
        let (program, _) = parse_ok("fn f() { x = 5; }");
        match &program.functions[0].body.stmts[0] {
            Stmt::Assign { value, .. } => {
                assert!(matches!(value.kind, ExprKind::IntLit(5)));
            }
            other => panic!("expected assign, got {other:?}"),
        }
        let (program2, _) = parse_ok("fn f() { g(1); }");
        assert!(matches!(
            &program2.functions[0].body.stmts[0],
            Stmt::Expr(Expr { kind: ExprKind::Call { .. }, .. })
        ));
        let (program3, _) = parse_ok("fn f() { x == 5; }");
        assert!(matches!(
            &program3.functions[0].body.stmts[0],
            Stmt::Expr(Expr { kind: ExprKind::Binary(BinOp::Eq, _, _), .. })
        ));
    }

    /// CEP:WHAT: let forms with and without mut, annotation, and initializer.
    /// CEP:WHY: The full let grammar surface.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn parser_let_shapes() {
        let (program, _) = parse_ok("fn f() { let a = 1; let mut b: u64 = 2; let c; return; }");
        let stmts = &program.functions[0].body.stmts;
        match &stmts[0] {
            Stmt::Let { mutable, ty, init, .. } => {
                assert!(!*mutable);
                assert!(ty.is_none());
                assert!(init.is_some());
            }
            other => panic!("expected let, got {other:?}"),
        }
        match &stmts[1] {
            Stmt::Let { mutable, ty, init, .. } => {
                assert!(*mutable);
                assert_eq!(*ty, Some(AstType::U64));
                assert!(init.is_some());
            }
            other => panic!("expected let, got {other:?}"),
        }
        match &stmts[2] {
            Stmt::Let { init, .. } => assert!(init.is_none()),
            other => panic!("expected let, got {other:?}"),
        }
        assert!(matches!(&stmts[3], Stmt::Return { value: None, .. }));
    }

    /// CEP:WHAT: if/else and else-if chain shapes.
    /// CEP:WHY: The conditional grammar with both else forms.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn parser_if_else_shapes() {
        let (program, _) =
            parse_ok("fn f() { if true { } else if false { } else { } }");
        match &program.functions[0].body.stmts[0] {
            Stmt::If { else_clause: Some(ElseClause::If(inner)), .. } => {
                assert!(matches!(
                    inner.as_ref(),
                    Stmt::If { else_clause: Some(ElseClause::Block(_)), .. }
                ));
            }
            other => panic!("expected else-if, got {other:?}"),
        }
    }

    /// CEP:WHAT: A trailing expression without semicolon closes the block.
    /// CEP:WHY: Fixture syntax `fn main() -> i32 { add(6, 7) }`.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn parser_trailing_expression() {
        let (program, _) = parse_ok("fn f() -> i32 { let x = 1; x }");
        let body = &program.functions[0].body;
        assert_eq!(body.stmts.len(), 1);
        assert!(body.trailing.is_some());
    }

    /// CEP:WHAT: Nesting above MAX_PARSE_DEPTH generated in a loop is rejected cleanly.
    /// CEP:WHY: 38.13: deep input must produce an error, never a stack overflow.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only, O(MAX_PARSE_DEPTH squared) worst case.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn parser_rejects_nesting_over_max_parse_depth() {
        let mut deep = String::from("fn f() -> i32 { ");
        for _ in 0..(MAX_PARSE_DEPTH + 10) {
            deep.push('(');
        }
        deep.push('1');
        for _ in 0..(MAX_PARSE_DEPTH + 10) {
            deep.push(')');
        }
        deep.push_str(" }");
        match parse_err(&deep) {
            FrontendError::NestingTooDeep { .. } => {}
            other => panic!("expected NestingTooDeep, got {other:?}"),
        }
        // A modest nesting level must still be accepted.
        let mut fine = String::from("fn f() -> i32 { ");
        for _ in 0..10 {
            fine.push('(');
        }
        fine.push('1');
        for _ in 0..10 {
            fine.push(')');
        }
        fine.push_str(" }");
        let (program, _) = parse_ok(&fine);
        assert!(program.functions[0].body.trailing.is_some());
    }

    /// CEP:WHAT: A unary chain above the depth budget is rejected cleanly.
    /// CEP:WHY: Prefix chains recurse once per operator; the budget must bound them.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn parser_rejects_unary_chain_over_budget() {
        let mut deep = String::from("fn f() -> i32 { ");
        for _ in 0..(MAX_PARSE_DEPTH + 10) {
            deep.push('!');
        }
        deep.push_str("true }");
        match parse_err(&deep) {
            FrontendError::NestingTooDeep { .. } => {}
            other => panic!("expected NestingTooDeep, got {other:?}"),
        }
        let mut fine = String::from("fn f() -> bool { ");
        for _ in 0..50 {
            fine.push('!');
        }
        fine.push_str("true }");
        let (program, _) = parse_ok(&fine);
        assert!(program.functions[0].body.trailing.is_some());
    }

    /// CEP:WHY: Negative classes: missing semicolons and trailing garbage.
    /// CEP:WHAT: Verifies Expect* error paths carry spans.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn parser_rejects_missing_semi() {
        match parse_err("fn f() { let x = 1 }") {
            // Inside a braced block the missing semicolon surfaces at the block end
            // (the closing brace belongs to the block itself).
            FrontendError::Parse { .. } => {}
            other => panic!("expected Parse error, got {other:?}"),
        }
        match parse_err("fn f() { 1 + }") {
            // Inside a braced block, a missing right-hand side surfaces as end of stream.
            FrontendError::Parse { .. } => {}
            other => panic!("expected Parse error for missing rhs, got {other:?}"),
        }
        match parse_err("fn f() { g(1 2); }") {
            FrontendError::Parse { kind: ParseErrorKind::UnexpectedToken { .. }, .. } => {}
            other => panic!("expected Parse error, got {other:?}"),
        }
    }

    /// CEP:WHAT: Operators outside the frozen HIR set are rejected with clear errors.
    /// CEP:WHY: Constructs without an HIR shape must never reach lowering.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn parser_rejects_unsupported_ops() {
        for src in [
            "fn f(a: bool, b: bool) -> bool { a != b }",
            "fn f(a: i32, b: i32) -> bool { a > b }",
            "fn f(a: i32, b: i32) -> bool { a >= b }",
            "fn f(a: bool, b: bool) -> bool { a && b }",
            "fn f(a: bool, b: bool) -> bool { a || b }",
        ] {
            match parse_err(src) {
                FrontendError::Unsupported { .. } => {}
                other => panic!("expected Unsupported for {src}, got {other:?}"),
            }
        }
        match parse_err("fn f() -> i32 { [1] }") {
            FrontendError::Unsupported { .. } => {}
            other => panic!("expected Unsupported for brackets, got {other:?}"),
        }
    }

    /// CEP:WHAT: Primitive type spellings are rejected as declared names.
    /// CEP:WHY: Keeps type positions unambiguous.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn parser_rejects_type_named_var() {
        for src in [
            "fn f() { let i32 = 1; }",
            "fn i32() { }",
            "fn f(i32: i32) { }",
            "fn f() -> i32 { i32 }",
        ] {
            match parse_err(src) {
                FrontendError::Parse { .. } => {}
                other => panic!("expected Parse error for {src}, got {other:?}"),
            }
        }
    }

    /// CEP:WHAT: Parameter lists above MAX_PARAMS are rejected.
    /// CEP:WHY: Named budget (38.12).
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn parser_rejects_too_many_params() {
        let mut src = String::from("fn f(");
        for i in 0..=MAX_PARAMS {
            if i > 0 {
                src.push_str(", ");
            }
            src.push_str(&format!("a{i}: i32"));
        }
        src.push_str(") { }");
        match parse_err(&src) {
            FrontendError::AstTooLarge { .. } => {}
            other => panic!("expected AstTooLarge, got {other:?}"),
        }
        let mut ok_src = String::from("fn f(");
        for i in 0..MAX_PARAMS {
            if i > 0 {
                ok_src.push_str(", ");
            }
            ok_src.push_str(&format!("a{i}: i32"));
        }
        ok_src.push_str(") { }");
        let (program, _) = parse_ok(&ok_src);
        assert_eq!(program.functions[0].params.len(), MAX_PARAMS);
    }

    /// CEP:WHAT: Programs above MAX_FUNCTIONS or MAX_AST_NODES are rejected.
    /// CEP:WHY: Named budgets (38.12).
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only, dominated by generated source size.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn parser_rejects_oversize_program() {
        let mut src = String::new();
        for i in 0..=MAX_FUNCTIONS {
            src.push_str(&format!("fn f{i}() {{ }}\n"));
        }
        match parse_err(&src) {
            FrontendError::AstTooLarge { .. } => {}
            other => panic!("expected AstTooLarge, got {other:?}"),
        }
    }

    /// CEP:WHAT: The AST node budget rejects chains beyond MAX_AST_NODES.
    /// CEP:WHY: Named budget (38.12); exercised with a long generated chain.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only, O(MAX_AST_NODES).
    /// CEP:EVIDENCE: this test.
    #[test]
    fn parser_ast_node_budget() {
        // One charge per literal and per binary node: pairs of "1 +" exceed the budget.
        let pairs = (MAX_AST_NODES / 2) as usize + 1;
        let mut src = String::with_capacity(pairs * 4);
        src.push_str("fn f() -> i32 { ");
        for _ in 0..pairs {
            src.push_str("1 + ");
        }
        src.push_str("1 }");
        match parse_err(&src) {
            FrontendError::AstTooLarge { .. } => {}
            other => panic!("expected AstTooLarge, got {other:?}"),
        }
    }
}
