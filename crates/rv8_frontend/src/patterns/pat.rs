// CEP:FILE: crates/rv8_frontend/src/patterns/pat.rs
// CEP:WHAT: The pattern IR consumed by the exhaustiveness engine: Pattern (wildcards,
//   bindings, boolean and integer literals, Option constructors), PatternType (the
//   scrutinee type space: bool, the ten integer widths, Option chains), integer kinds
//   with literal range bounds, iterative depth measurement, and type conformance checking.
// CEP:WHY: The frontend AST has no pattern nodes yet, so this standalone IR is the exact
//   shape a future match-desugaring pass will produce; anchoring it on IrType-adjacent
//   widths keeps the engine honest about what the frontend models.
// CEP:CLASS: HPC-1
// CEP:STATUS: complete
// CEP:FAILURE: check_conformance returns a PatternError for every pattern that does not
//   conform to the scrutinee type or whose integer literal is out of range; never panics.
// CEP:ASSUMES: patterns arrive from the AST-side producer with SymbolId binding names;
//   user-defined enums do not exist in the subset, so Option is the only closed
//   multi-constructor type.
// CEP:COST: depth measurement is O(depth) iterative; conformance is O(pattern nodes) with
//   recursion bounded by the caller's MAX_PATTERN_DEPTH validation.
// CEP:EVIDENCE: integration tests pattern_exhaustiveness.rs (conformance, ranges, depth).

use super::diag::{PatternError, TypeShape};
use rv8_core::interner::SymbolId;
use rv8_ir::son::IrType;
use std::fmt;

/// CEP:WHAT: One integer kind of the IR scalar set, with its width and signedness.
/// CEP:WHY: Literal range validation and constructor enumeration need the exact width.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: POD enum.
/// CEP:EVIDENCE: test patterns_from_ir_maps_scalar_widths in pattern_exhaustiveness.rs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum IntKind {
    I8,
    I16,
    I32,
    I64,
    I128,
    U8,
    U16,
    U32,
    U64,
    U128,
}

/// CEP:WHAT: Bit position of the sign bit of the widest signed kind.
/// CEP:WHY: Two's complement range bounds for widths below the widest are computed from it.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: unit test rv8_frontend::patterns::pat::tests::int_kind_bounds.
const I128_SIGN_SHIFT: u8 = 127;

/// CEP:WHAT: Width of the widest integer kinds.
/// CEP:WHY: The widest signed and unsigned kinds saturate i128 range checks.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constant.
/// CEP:EVIDENCE: unit test rv8_frontend::patterns::pat::tests::int_kind_bounds.
const WIDEST_BITS: u8 = 128;

/// CEP:WHAT: The scalar bit widths of the integer kinds, named per the magic-number law.
/// CEP:WHY: IntKind::bits maps kinds onto widths without inline numeric literals (Law 7).
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: compile-time constants.
/// CEP:EVIDENCE: unit test rv8_frontend::patterns::pat::tests::int_kind_bounds.
const BITS_8: u8 = 8;
const BITS_16: u8 = 16;
const BITS_32: u8 = 32;
const BITS_64: u8 = 64;
const BITS_128: u8 = 128;

impl IntKind {
    /// CEP:WHAT: Maps one integer IrType onto its kind.
    /// CEP:WHY: The scrutinee type arrives from inference as an IrType scalar.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: returns None for non-integer types.
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1) match.
    /// CEP:EVIDENCE: test patterns_from_ir_maps_scalar_widths.
    pub fn from_ir(ty: IrType) -> Option<IntKind> {
        match ty {
            IrType::I8 => Some(IntKind::I8),
            IrType::I16 => Some(IntKind::I16),
            IrType::I32 => Some(IntKind::I32),
            IrType::I64 => Some(IntKind::I64),
            IrType::I128 => Some(IntKind::I128),
            IrType::U8 => Some(IntKind::U8),
            IrType::U16 => Some(IntKind::U16),
            IrType::U32 => Some(IntKind::U32),
            IrType::U64 => Some(IntKind::U64),
            IrType::U128 => Some(IntKind::U128),
            _ => None,
        }
    }

    /// CEP:WHAT: Reports the width in bits.
    /// CEP:WHY: Range bounds and diagnostics need the width.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1) match.
    /// CEP:EVIDENCE: test patterns_int_kind_bounds.
    pub fn bits(self) -> u8 {
        match self {
            IntKind::I8 | IntKind::U8 => BITS_8,
            IntKind::I16 | IntKind::U16 => BITS_16,
            IntKind::I32 | IntKind::U32 => BITS_32,
            IntKind::I64 | IntKind::U64 => BITS_64,
            IntKind::I128 | IntKind::U128 => BITS_128,
        }
    }

    /// CEP:WHAT: Reports signedness.
    /// CEP:WHY: Range bounds differ between signed and unsigned kinds.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: test patterns_int_kind_bounds.
    pub fn is_signed(self) -> bool {
        matches!(self, IntKind::I8 | IntKind::I16 | IntKind::I32 | IntKind::I64 | IntKind::I128)
    }

    /// CEP:WHAT: Reports whether one literal value is representable in the kind.
    /// CEP:WHY: Literal patterns outside the scrutinee range are rejected, not wrapped.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: literals are i128, so U128 values above i128::MAX are unreachable
    ///   through this IR and every non-negative i128 is accepted for U128.
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: tests patterns_int_kind_bounds and patterns_literal_out_of_range.
    pub fn contains(self, value: i128) -> bool {
        let bits = self.bits();
        if self.is_signed() {
            let half = bits - 1;
            let low = match half >= I128_SIGN_SHIFT {
                true => i128::MIN,
                false => -(1i128 << half),
            };
            let high = match half >= I128_SIGN_SHIFT {
                true => i128::MAX,
                false => (1i128 << half) - 1,
            };
            value >= low && value <= high
        } else {
            value >= 0
                && match bits >= WIDEST_BITS {
                    true => true,
                    false => {
                        let high = if bits >= I128_SIGN_SHIFT {
                            i128::MAX
                        } else {
                            (1i128 << bits) - 1
                        };
                        value <= high
                    }
                }
        }
    }
}

impl fmt::Display for IntKind {
    /// CEP:WHAT: Renders the kind as its source spelling.
    /// CEP:WHY: Deterministic diagnostics (38.15) need one canonical text form.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: cold-path formatting only.
    /// CEP:EVIDENCE: test patterns_display_roundtrip.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            IntKind::I8 => "i8",
            IntKind::I16 => "i16",
            IntKind::I32 => "i32",
            IntKind::I64 => "i64",
            IntKind::I128 => "i128",
            IntKind::U8 => "u8",
            IntKind::U16 => "u16",
            IntKind::U32 => "u32",
            IntKind::U64 => "u64",
            IntKind::U128 => "u128",
        };
        write!(f, "{name}")
    }
}

/// CEP:WHAT: The scrutinee type space: booleans, integer widths, and Option chains.
/// CEP:WHY: Constructor enumeration is defined per scrutinee type; Option is the only
///   closed multi-constructor type the frontend models.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: owned Box on the Option variant only.
/// CEP:EVIDENCE: test patterns_from_ir_maps_scalar_widths.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum PatternType {
    Bool,
    Int(IntKind),
    Option(Box<PatternType>),
}

impl PatternType {
    /// CEP:WHAT: Builds Option of an inner type.
    /// CEP:WHY: Tests and future producers need a readable constructor.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: one Box.
    /// CEP:EVIDENCE: this crate's tests.
    pub fn option_of(inner: PatternType) -> PatternType {
        PatternType::Option(Box::new(inner))
    }

    /// CEP:WHAT: Maps a scalar IrType onto the pattern type space.
    /// CEP:WHY: The integration hook receives scrutinee types from inference as IrType.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: returns None for floats, characters, pointers, and unit, which have no
    ///   pattern constructors in this subset.
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: test patterns_from_ir_maps_scalar_widths.
    pub fn from_ir(ty: IrType) -> Option<PatternType> {
        match ty {
            IrType::Bool => Some(PatternType::Bool),
            other => IntKind::from_ir(other).map(PatternType::Int),
        }
    }

    /// CEP:WHAT: Reports the coarse shape used by structured type-mismatch errors.
    /// CEP:WHY: PatternError carries shapes instead of owned types so errors stay cheap.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: test patterns_type_mismatch_litint_on_bool.
    pub fn shape(&self) -> TypeShape {
        match self {
            PatternType::Bool => TypeShape::Bool,
            PatternType::Int(_) => TypeShape::Int,
            PatternType::Option(_) => TypeShape::Option,
        }
    }

    /// CEP:WHAT: Measures the Option nesting depth of the scrutinee type iteratively.
    /// CEP:WHY: Engine recursion depth is bounded by type depth as well as pattern depth,
    ///   so a deeply nested scrutinee type is a structural rejection, not a stack overflow.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(depth), iterative.
    /// CEP:EVIDENCE: test patterns_deep_type_over_budget_rejects.
    pub fn depth_iterative(&self) -> u32 {
        let mut depth = 1u32;
        let mut current: &PatternType = self;
        while let PatternType::Option(inner) = current {
            depth = depth.saturating_add(1);
            current = inner;
        }
        depth
    }
}

impl fmt::Display for PatternType {
    /// CEP:WHAT: Renders the type in source-like spelling.
    /// CEP:WHY: Deterministic diagnostics need one canonical text form.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: cold-path formatting only.
    /// CEP:EVIDENCE: test patterns_display_roundtrip.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PatternType::Bool => write!(f, "bool"),
            PatternType::Int(kind) => write!(f, "{kind}"),
            PatternType::Option(inner) => write!(f, "Option<{inner}>"),
        }
    }
}

/// CEP:WHAT: One pattern: wildcard, binding, boolean or integer literal, or an Option
///   constructor application.
/// CEP:WHY: This is the node set the match desugaring will produce and the engine consumes;
///   every node has at most one child, so depth measurement is a linear walk.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: binding names are interner symbols; user-defined struct and tuple patterns
///   do not exist in the subset.
/// CEP:COST: one Box per OptionSome level.
/// CEP:EVIDENCE: integration tests pattern_exhaustiveness.rs.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Pattern {
    Wildcard,
    Binding { name: SymbolId },
    LitBool(bool),
    LitInt(i128),
    OptionNone,
    OptionSome(Box<Pattern>),
}

impl Pattern {
    /// CEP:WHAT: Iteratively deallocates an OptionSome chain in place, leaving Wildcard.
    /// CEP:WHY: A rejected over-budget pattern is still owned by the caller, and the
    ///   default derived Drop recurses one frame per Some level; this utility releases
    ///   arbitrarily deep rejected input without a stack overflow.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: callers that accept patterns from untrusted sources call this on
    ///   rejected over-budget input instead of dropping it directly.
    /// CEP:COST: O(depth), iterative; one stack slot.
    /// CEP:EVIDENCE: test patterns_deep_input_over_budget_rejects releases a 200_000-deep
    ///   pattern through this path without a stack overflow.
    pub fn dispose_iterative(&mut self) {
        loop {
            let child: Option<Pattern> = match self {
                Pattern::OptionSome(inner) => {
                    Some(std::mem::replace(&mut **inner, Pattern::Wildcard))
                }
                _ => None,
            };
            match child {
                Some(next) => {
                    let old = std::mem::replace(self, next);
                    drop(old);
                }
                None => break,
            }
        }
    }

    /// CEP:WHAT: Builds a binding pattern.
    /// CEP:WHY: Tests and future producers need a readable constructor.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: this crate's tests.
    pub fn binding(name: u32) -> Pattern {
        Pattern::Binding {
            name: SymbolId(name),
        }
    }

    /// CEP:WHAT: Builds an integer literal pattern.
    /// CEP:WHY: Tests and future producers need a readable constructor.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: this crate's tests.
    pub fn lit_int(value: i128) -> Pattern {
        Pattern::LitInt(value)
    }

    /// CEP:WHAT: Builds Some of an inner pattern.
    /// CEP:WHY: Tests and future producers need a readable constructor.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: one Box.
    /// CEP:EVIDENCE: this crate's tests.
    pub fn some(inner: Pattern) -> Pattern {
        Pattern::OptionSome(Box::new(inner))
    }

    /// CEP:WHAT: Reports whether the pattern is wildcard-like (Wildcard or Binding), which
    ///   covers every value of its column.
    /// CEP:WHY: Matrix defaulting and usefulness treat both identically.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_frontend::patterns::matrix::tests::default_matrix_keeps_wildcards.
    pub fn is_wildcard_like(&self) -> bool {
        matches!(self, Pattern::Wildcard | Pattern::Binding { .. })
    }

    /// CEP:WHAT: Measures the nesting depth (nodes on the longest chain) iteratively.
    /// CEP:WHY: The fail-closed law requires that untrusted deep input never drives
    ///   recursion; this walk uses no stack frames proportional to depth.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(depth), iterative.
    /// CEP:EVIDENCE: test patterns_deep_input_over_budget_rejects proves a 100_000-deep
    ///   pattern is measured without overflowing the stack.
    pub fn depth_iterative(&self) -> u32 {
        let mut depth = 1u32;
        let mut current: &Pattern = self;
        while let Pattern::OptionSome(inner) = current {
            depth = depth.saturating_add(1);
            current = inner;
        }
        depth
    }
}

impl fmt::Display for Pattern {
    /// CEP:WHAT: Renders the pattern in source-like spelling; bindings render as `x{id}`
    ///   because binding names never appear in missing-pattern witnesses.
    /// CEP:WHY: Deterministic diagnostics need one canonical text form.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: cold-path formatting only.
    /// CEP:EVIDENCE: test patterns_display_roundtrip.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Pattern::Wildcard => write!(f, "_"),
            Pattern::Binding { name } => write!(f, "x{}", name.0),
            Pattern::LitBool(value) => write!(f, "{value}"),
            Pattern::LitInt(value) => write!(f, "{value}"),
            Pattern::OptionNone => write!(f, "None"),
            Pattern::OptionSome(inner) => write!(f, "Some({inner})"),
        }
    }
}

/// CEP:WHAT: Checks that one pattern conforms to the scrutinee type, including literal
///   ranges, before any engine work.
/// CEP:WHY: Fail-closed law (38.6): ill-typed patterns are rejected with structured errors,
///   never coerced ("do not just pass anything").
/// CEP:STATUS: complete
/// CEP:FAILURE: TypeMismatch for constructor/literal shape conflicts; LiteralOutOfRange for
///   an integer literal outside the column kind.
/// CEP:ASSUMES: the caller has already bounded the depth at MAX_PATTERN_DEPTH, so this
///   recursion uses at most MAX_PATTERN_DEPTH frames.
/// CEP:COST: O(pattern nodes).
/// CEP:EVIDENCE: tests patterns_type_mismatch_litint_on_bool, patterns_type_mismatch_none_on_int,
///   patterns_literal_out_of_range.
pub fn check_conformance(pat: &Pattern, ty: &PatternType) -> Result<(), PatternError> {
    match (pat, ty) {
        (Pattern::Wildcard, _) | (Pattern::Binding { .. }, _) => Ok(()),
        (Pattern::LitBool(_), PatternType::Bool) => Ok(()),
        (Pattern::LitBool(_), other) => Err(PatternError::TypeMismatch {
            context: "pattern",
            expected: other.shape(),
            found: TypeShape::Bool,
        }),
        (Pattern::LitInt(value), PatternType::Int(kind)) => {
            if kind.contains(*value) {
                Ok(())
            } else {
                Err(PatternError::LiteralOutOfRange {
                    value: *value,
                    ty: *kind,
                })
            }
        }
        (Pattern::LitInt(_), other) => Err(PatternError::TypeMismatch {
            context: "pattern",
            expected: other.shape(),
            found: TypeShape::Int,
        }),
        (Pattern::OptionNone, PatternType::Option(_)) => Ok(()),
        (Pattern::OptionNone, other) => Err(PatternError::TypeMismatch {
            context: "pattern",
            expected: other.shape(),
            found: TypeShape::Option,
        }),
        (Pattern::OptionSome(inner), PatternType::Option(item)) => check_conformance(inner, item),
        (Pattern::OptionSome(_), other) => Err(PatternError::TypeMismatch {
            context: "pattern",
            expected: other.shape(),
            found: TypeShape::Option,
        }),
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// CEP:WHAT: Depth measurement counts constructor levels and is iterative.
    /// CEP:WHY: The budget contract of MAX_PATTERN_DEPTH depends on this definition.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn depth_counts_constructor_levels() {
        assert_eq!(Pattern::Wildcard.depth_iterative(), 1);
        assert_eq!(Pattern::OptionNone.depth_iterative(), 1);
        assert_eq!(Pattern::some(Pattern::Wildcard).depth_iterative(), 2);
        let deep = Pattern::some(Pattern::some(Pattern::lit_int(7)));
        assert_eq!(deep.depth_iterative(), 3);
    }

    /// CEP:WHAT: Integer kind range bounds across signed and unsigned widths.
    /// CEP:WHY: Literal validation must reject values that would wrap.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn int_kind_bounds() {
        assert!(IntKind::U8.contains(0) && IntKind::U8.contains(255));
        assert!(!IntKind::U8.contains(256) && !IntKind::U8.contains(-1));
        assert!(IntKind::I8.contains(-128) && IntKind::I8.contains(127));
        assert!(!IntKind::I8.contains(128) && !IntKind::I8.contains(-129));
        assert!(IntKind::I128.contains(i128::MIN) && IntKind::I128.contains(i128::MAX));
        assert!(IntKind::U128.contains(0));
        assert!(!IntKind::U128.contains(-1));
        assert!(IntKind::U32.contains(4_294_967_295));
        assert!(!IntKind::U32.contains(4_294_967_296));
    }
}
