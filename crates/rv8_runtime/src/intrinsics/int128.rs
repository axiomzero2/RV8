// CEP:FILE: crates/rv8_runtime/src/intrinsics/int128.rs
// CEP:WHAT: Software 128-bit integer division and remainder: u128/u128 and i128/i128 via
//   shift-and-subtract over 64-bit limbs, with checked variants returning typed errors and
//   unchecked variants panicking exactly where std's operators panic.
// CEP:WHY: design.md requires software implementations of 128-bit division and modulo for
//   targets without hardware support; most targets lack a 128/128 divide, so the runtime
//   provides compiler-rt replacement semantics.
// CEP:CLASS: HPC-1
// CEP:STATUS: complete
// CEP:FAILURE: checked_* return IntMathError::DivideByZero or IntMathError::Overflow;
//   udiv_128, urem_128, idiv_128, and irem_128 panic with the std messages on the same
//   inputs std's operators panic on; no other panic paths.
// CEP:ASSUMES: no u128 division or remainder operator is used anywhere in this module; only
//   shifts, adds, subtracts, compares, and bitwise operations on u64 and u128.
// CEP:COST: O(128) limb steps per division; O(1) overhead for the signed wrappers.
// CEP:EVIDENCE: unit tests in this module and integration tests runtime_foundation.rs
//   (bit-exact against native u128/i128 operators on the host).

use core::fmt;

/// CEP:WHAT: The failure type of checked 128-bit arithmetic.
/// CEP:WHY: Fail-closed law (38.6): divide by zero and MIN/-1 overflow are typed errors in
///   the checked API.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; this type enumerates the failures.
/// CEP:ASSUMES: none
/// CEP:COST: POD.
/// CEP:EVIDENCE: test checked_variants_fail_closed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum IntMathError {
    DivideByZero,
    Overflow,
}

impl fmt::Display for IntMathError {
    /// CEP:WHAT: Renders the error in one stable line.
    /// CEP:WHY: Deterministic diagnostics need one canonical text form.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: cold-path formatting only.
    /// CEP:EVIDENCE: this module's tests.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IntMathError::DivideByZero => write!(f, "attempt to divide by zero"),
            IntMathError::Overflow => write!(f, "attempt to divide with overflow"),
        }
    }
}

/// CEP:WHAT: A 128-bit value as two 64-bit limbs, high first.
/// CEP:WHY: design.md pins the shift-and-subtract loop to 64-bit limbs; this type carries
///   the remainder and quotient state.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: POD.
/// CEP:EVIDENCE: test u128_div_matches_native_edges.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Limbs {
    hi: u64,
    lo: u64,
}

impl Limbs {
    /// CEP:WHAT: Splits one u128 into limbs.
    /// CEP:WHY: The loop state lives in limbs per design.md.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: this module's tests.
    fn of(value: u128) -> Limbs {
        Limbs {
            hi: (value >> 64) as u64,
            lo: value as u64,
        }
    }

    /// CEP:WHAT: Joins the limbs back into one u128.
    /// CEP:WHY: Results are returned as u128 values.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: this module's tests.
    fn to_u128(self) -> u128 {
        ((self.hi as u128) << 64) | self.lo as u128
    }

    /// CEP:WHAT: Shifts the limbs left one bit, bringing in one dividend bit.
    /// CEP:WHY: The shift half of shift-and-subtract.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: this module's tests.
    fn shift_in(&mut self, bit: u64) {
        self.hi = (self.hi << 1) | (self.lo >> 63);
        self.lo = (self.lo << 1) | (bit & 1);
    }

    /// CEP:WHAT: Reports whether the limbs are at least the divisor.
    /// CEP:WHY: The comparison half of shift-and-subtract.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: this module's tests.
    fn at_least(&self, d: Limbs) -> bool {
        (self.hi > d.hi) || (self.hi == d.hi && self.lo >= d.lo)
    }

    /// CEP:WHAT: Subtracts the divisor in place (two's-complement limb borrow).
    /// CEP:WHY: The subtract half of shift-and-subtract; called only when at_least holds.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: self >= d
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: this module's tests.
    fn subtract(&mut self, d: Limbs) {
        let (lo, borrow) = self.lo.overflowing_sub(d.lo);
        self.lo = lo;
        self.hi = self.hi.wrapping_sub(d.hi).wrapping_sub(borrow as u64);
    }

    /// CEP:WHAT: Sets one quotient bit at limb position.
    /// CEP:WHY: The quotient accumulates bit by bit.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: bit_index < 128
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: this module's tests.
    fn set_quotient_bit(q: &mut Limbs, bit_index: u32) {
        if bit_index >= 64 {
            q.hi |= 1u64 << (bit_index - 64);
        } else {
            q.lo |= 1u64 << bit_index;
        }
    }
}

/// CEP:WHAT: Divides one u128 by another, returning quotient and remainder; the algorithm
///   is binary shift-and-subtract over 128 iterations with 64-bit limb state.
/// CEP:WHY: The software replacement for the 128/128 hardware divide.
/// CEP:STATUS: complete
/// CEP:FAILURE: IntMathError::DivideByZero when d is zero.
/// CEP:ASSUMES: none
/// CEP:COST: O(128) limb steps, O(1) space.
/// CEP:EVIDENCE: tests u128_div_matches_native_edges, u128_div_alternating_limbs, and the
///   host fuzz test udiv_fuzz_matches_native.
pub fn checked_udivmod_128(n: u128, d: u128) -> Result<(u128, u128), IntMathError> {
    if d == 0 {
        return Err(IntMathError::DivideByZero);
    }
    let dividend = Limbs::of(n);
    let divisor = Limbs::of(d);
    let mut quotient = Limbs {
        hi: 0,
        lo: 0,
    };
    let mut remainder = Limbs {
        hi: 0,
        lo: 0,
    };
    for bit_index in (0..128u32).rev() {
        let bit = if bit_index >= 64 {
            (dividend.hi >> (bit_index - 64)) & 1
        } else {
            (dividend.lo >> bit_index) & 1
        };
        remainder.shift_in(bit);
        if remainder.at_least(divisor) {
            remainder.subtract(divisor);
            Limbs::set_quotient_bit(&mut quotient, bit_index);
        }
    }
    Ok((quotient.to_u128(), remainder.to_u128()))
}

/// CEP:WHAT: Checked u128 quotient.
/// CEP:WHY: The checked API surface for generated fallible code.
/// CEP:STATUS: complete
/// CEP:FAILURE: IntMathError::DivideByZero.
/// CEP:ASSUMES: none
/// CEP:COST: one division.
/// CEP:EVIDENCE: test checked_variants_fail_closed.
pub fn checked_udiv_128(n: u128, d: u128) -> Result<u128, IntMathError> {
    checked_udivmod_128(n, d).map(|(q, _)| q)
}

/// CEP:WHAT: Checked u128 remainder.
/// CEP:WHY: The checked API surface for generated fallible code.
/// CEP:STATUS: complete
/// CEP:FAILURE: IntMathError::DivideByZero.
/// CEP:ASSUMES: none
/// CEP:COST: one division.
/// CEP:EVIDENCE: test checked_variants_fail_closed.
pub fn checked_urem_128(n: u128, d: u128) -> Result<u128, IntMathError> {
    checked_udivmod_128(n, d).map(|(_, r)| r)
}

/// CEP:WHAT: Checked i128 division with std overflow semantics (MIN / -1 overflows).
/// CEP:WHY: The checked API surface for signed 128-bit code.
/// CEP:STATUS: complete
/// CEP:FAILURE: IntMathError::DivideByZero or IntMathError::Overflow (MIN / -1).
/// CEP:ASSUMES: none
/// CEP:COST: one unsigned division plus sign fixups.
/// CEP:EVIDENCE: test i128_div_signed_cases.
pub fn checked_idivmod_128(n: i128, d: i128) -> Result<(i128, i128), IntMathError> {
    if d == 0 {
        return Err(IntMathError::DivideByZero);
    }
    if n == i128::MIN && d == -1 {
        return Err(IntMathError::Overflow);
    }
    let (uq, ur) = checked_udivmod_128(n.unsigned_abs(), d.unsigned_abs())?;
    let quotient_negative = (n < 0) != (d < 0);
    let quotient = match quotient_negative {
        true => (uq as i128).wrapping_neg(),
        false => uq as i128,
    };
    let remainder = match n < 0 {
        true => (ur as i128).wrapping_neg(),
        false => ur as i128,
    };
    Ok((quotient, remainder))
}

/// CEP:WHAT: Checked i128 quotient.
/// CEP:WHY: The checked API surface for signed 128-bit code.
/// CEP:STATUS: complete
/// CEP:FAILURE: IntMathError::DivideByZero or IntMathError::Overflow.
/// CEP:ASSUMES: none
/// CEP:COST: one signed division.
/// CEP:EVIDENCE: test i128_div_signed_cases.
pub fn checked_idiv_128(n: i128, d: i128) -> Result<i128, IntMathError> {
    checked_idivmod_128(n, d).map(|(q, _)| q)
}

/// CEP:WHAT: Checked i128 remainder.
/// CEP:WHY: The checked API surface for signed 128-bit code.
/// CEP:STATUS: complete
/// CEP:FAILURE: IntMathError::DivideByZero.
/// CEP:ASSUMES: none
/// CEP:COST: one signed division.
/// CEP:EVIDENCE: test i128_div_signed_cases.
pub fn checked_irem_128(n: i128, d: i128) -> Result<i128, IntMathError> {
    checked_idivmod_128(n, d).map(|(_, r)| r)
}

/// CEP:WHAT: The u128 quotient with std panic semantics (divide by zero panics).
/// CEP:WHY: Generated code using the `/` operator on u128 needs the std-mirroring panic.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics on d == 0 with the std message, mirroring the documented
///   unwrap-style exception.
/// CEP:ASSUMES: none
/// CEP:COST: one division.
/// CEP:EVIDENCE: test unchecked_div_zero_panics_like_std.
pub fn udiv_128(n: u128, d: u128) -> u128 {
    match checked_udiv_128(n, d) {
        Ok(q) => q,
        Err(IntMathError::DivideByZero) => panic!("attempt to divide by zero"),
        Err(IntMathError::Overflow) => unreachable!("u128 division cannot overflow"),
    }
}

/// CEP:WHAT: The u128 remainder with std panic semantics.
/// CEP:WHY: Generated code using the `%` operator on u128.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics on d == 0, mirroring std.
/// CEP:ASSUMES: none
/// CEP:COST: one division.
/// CEP:EVIDENCE: test unchecked_div_zero_panics_like_std.
pub fn urem_128(n: u128, d: u128) -> u128 {
    match checked_urem_128(n, d) {
        Ok(r) => r,
        Err(IntMathError::DivideByZero) => panic!("attempt to divide by zero"),
        Err(IntMathError::Overflow) => unreachable!("u128 remainder cannot overflow"),
    }
}

/// CEP:WHAT: The i128 quotient with std panic semantics (zero divisor, MIN / -1).
/// CEP:WHY: Generated code using the `/` operator on i128.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics mirroring std on both failure classes.
/// CEP:ASSUMES: none
/// CEP:COST: one division.
/// CEP:EVIDENCE: test unchecked_div_zero_panics_like_std.
pub fn idiv_128(n: i128, d: i128) -> i128 {
    match checked_idiv_128(n, d) {
        Ok(q) => q,
        Err(IntMathError::DivideByZero) => panic!("attempt to divide by zero"),
        Err(IntMathError::Overflow) => panic!("attempt to divide with overflow"),
    }
}

/// CEP:WHAT: The i128 remainder with std panic semantics.
/// CEP:WHY: Generated code using the `%` operator on i128.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics mirroring std on both failure classes.
/// CEP:ASSUMES: none
/// CEP:COST: one division.
/// CEP:EVIDENCE: test unchecked_div_zero_panics_like_std.
pub fn irem_128(n: i128, d: i128) -> i128 {
    match checked_irem_128(n, d) {
        Ok(r) => r,
        Err(IntMathError::DivideByZero) => panic!("attempt to divide by zero"),
        Err(IntMathError::Overflow) => panic!("attempt to divide with overflow"),
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// CEP:WHAT: u128 division matches native operators across edge cases.
    /// CEP:WHY: Bit-exactness against the host is the acceptance bar.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: host u128 division is IEEE-exact integer division.
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn u128_div_matches_native_edges() {
        let cases: [(u128, u128); 10] = [
            (0, 1),
            (1, 1),
            (0, u128::MAX),
            (1, u128::MAX),
            (u128::MAX, u128::MAX),
            (u128::MAX, 1),
            (u128::MAX, 2),
            (1u128 << 127, 2),
            (1u128 << 127, 1u128 << 64),
            (u128::MAX, 1u128 << 100),
        ];
        for (n, d) in cases {
            assert_eq!(udiv_128(n, d), n / d, "n={n} d={d}");
            assert_eq!(urem_128(n, d), n % d, "n={n} d={d}");
            assert_eq!(checked_udivmod_128(n, d), Ok((n / d, n % d)));
        }
    }

    /// CEP:WHAT: Alternating and top-bit-set limb patterns divide correctly.
    /// CEP:WHY: Limb-boundary carries are the classic failure mode.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn u128_div_alternating_limbs() {
        let alternating = 0xAAAA_AAAA_AAAA_AAAA_5555_5555_5555_5555u128;
        let top_set = 0xFFFF_FFFF_FFFF_FFFF_FFFF_FFFF_FFFF_FFFFu128 >> 7;
        for d in [
            3u128,
            0xFFFF_FFFF_FFFF_FFFFu128,
            0x1_0000_0000_0000_0001u128,
            0xDEAD_BEEF_CAFE_F00Du128,
        ] {
            assert_eq!(udiv_128(alternating, d), alternating / d, "d={d}");
            assert_eq!(urem_128(alternating, d), alternating % d, "d={d}");
            assert_eq!(udiv_128(top_set, d), top_set / d, "d={d}");
            assert_eq!(urem_128(top_set, d), top_set % d, "d={d}");
        }
        // Quotient reconstructs the dividend: q * d + r == n (using native mul/add).
        let n = 0x1234_5678_9ABC_DEF0_1122_3344_5566_7788u128;
        let d = 0x9E37_79B9_7F4A_7C15u128;
        let q = udiv_128(n, d);
        let r = urem_128(n, d);
        assert_eq!(q.wrapping_mul(d).wrapping_add(r), n);
    }

    /// CEP:WHAT: Checked variants return typed errors for zero divisors and MIN/-1.
    /// CEP:WHY: The fail-closed checked API contract.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn checked_variants_fail_closed() {
        assert_eq!(
            checked_udivmod_128(1, 0),
            Err(IntMathError::DivideByZero)
        );
        assert_eq!(checked_udiv_128(1, 0), Err(IntMathError::DivideByZero));
        assert_eq!(checked_urem_128(1, 0), Err(IntMathError::DivideByZero));
        assert_eq!(
            checked_idiv_128(i128::MIN, -1),
            Err(IntMathError::Overflow)
        );
        assert_eq!(checked_irem_128(i128::MIN, -1), Err(IntMathError::Overflow));
        assert_eq!(
            checked_idiv_128(5, 0),
            Err(IntMathError::DivideByZero)
        );
    }

    /// CEP:WHAT: Signed i128 division matches native operators including sign handling.
    /// CEP:WHY: The signed wrapper contract.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn i128_div_signed_cases() {
        let cases: [(i128, i128); 12] = [
            (7, 2),
            (-7, 2),
            (7, -2),
            (-7, -2),
            (0, 5),
            (i128::MIN, 1),
            (i128::MIN, 2),
            (i128::MAX, i128::MIN),
            (i128::MIN, i128::MAX),
            (12345, 6789),
            (-12345, 6789),
            (1, -1),
        ];
        for (n, d) in cases {
            assert_eq!(idiv_128(n, d), n / d, "n={n} d={d}");
            assert_eq!(irem_128(n, d), n % d, "n={n} d={d}");
        }
        // i128::MIN / 1 is representable and equals itself.
        assert_eq!(checked_idivmod_128(i128::MIN, 1), Ok((i128::MIN, 0)));
    }

    /// CEP:WHAT: The unchecked wrappers panic on the same inputs std's operators panic on.
    /// CEP:WHY: The documented std-mirroring panic exception.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics by design (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    #[should_panic(expected = "attempt to divide by zero")]
    fn unchecked_div_zero_panics_like_std() {
        let _ = udiv_128(1, 0);
    }

    /// CEP:WHAT: i128 MIN / -1 overflows and the unchecked wrapper panics like std.
    /// CEP:WHY: The signed overflow panic class.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics by design (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    #[should_panic(expected = "attempt to divide with overflow")]
    fn unchecked_min_over_neg_one_panics() {
        let _ = idiv_128(i128::MIN, -1);
    }

    /// CEP:WHAT: Deterministic pseudo-random u128 pairs match native division exactly.
    /// CEP:WHY: Broad correctness evidence beyond hand-picked edges, reproducibly.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: the xorshift generator is deterministic across runs.
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn udiv_fuzz_matches_native() {
        let mut state: u128 = 0x243F_6A88_85A3_08D3_1319_8A2E_0370_7344;
        for round in 0..512u32 {
            state ^= state << 29;
            state ^= state >> 71;
            state ^= state << 97;
            let n = state;
            let d = ((state >> 64) | (round as u128 + 1)).max(1);
            let (q, r) = checked_udivmod_128(n, d).expect("nonzero divisor");
            assert_eq!(q, n / d, "round {round}");
            assert_eq!(r, n % d, "round {round}");
            assert_eq!(q.wrapping_mul(d).wrapping_add(r), n);
        }
    }
}
