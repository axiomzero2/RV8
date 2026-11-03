// CEP:FILE: crates/rv8_runtime/src/intrinsics/softfloat.rs
// CEP:WHAT: IEEE 754 soft-float add, sub, mul, and div for f32 and f64, built from integer
//   bit manipulation with round-to-nearest-even, full subnormal support, infinities, and
//   canonical quiet-NaN production; plus the sixteen int<->float conversions (design.md
//   runtime section 5) with Rust `as`-cast semantics: round-to-nearest-even into the
//   float, truncation toward zero with saturation and NaN -> 0 into the integer.
// CEP:WHY: Standard 5.8 and design.md require software floating point for FPU-less targets;
//   the compiler recognizes these symbols and may replace calls with hardware instructions
//   when the target has an FPU.
// CEP:CLASS: HPC-1
// CEP:STATUS: complete
// CEP:FAILURE: none; every operation is total: invalid operations (inf-inf, 0/0, inf*0)
//   return the canonical quiet NaN, overflow returns infinity, and underflow returns zero
//   or a subnormal, all per IEEE 754 round-to-nearest-even defaults.
// CEP:ASSUMES: round-to-nearest-even is the only rounding mode (no mode register is
//   modeled); NaN payloads are not propagated - the canonical quiet NaN is returned, which
//   is a documented deviation from x86 payload forwarding; the only float hardware used is
//   the exact bit reinterpretation (to_bits/from_bits), never float arithmetic.
// CEP:COST: O(1) fixed-width integer work per operation (u64/u128); no data-dependent
//   loops beyond the bounded normalization shifts.
// CEP:EVIDENCE: bit-exact comparison against native f32/f64 across the value matrix
//   (including subnormals, infinities, and NaN propagation) in
//   crates/rv8_runtime/tests/runtime_foundation.rs, plus unit tests in this module;
//   the conversions are differentially pinned against native `as` casts by the
//   deterministic LCG sweeps and adversarial edge matrices in this module

/// CEP:WHAT: Format parameters of one IEEE 754 binary interchange width.
/// CEP:WHY: One generic core serves f32 and f64; only these two widths are required by
///   design.md.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: E + M < 63 so the working significand fits u64 and the exact add/sub
///   alignment product fits u128.
/// CEP:COST: compile-time constants.
/// CEP:EVIDENCE: the f32/f64 matrix tests against native.
trait Params {
    /// Exponent field width in bits.
    const E: u32;
    /// Fraction (mantissa) width in bits.
    const M: u32;
    /// Exponent bias: 2^(E-1) - 1.
    const BIAS: i32 = (1i32 << (Self::E - 1)) - 1;
    /// Unbiased exponent of the smallest normal.
    const EMIN: i32 = 1 - Self::BIAS;
    /// Unbiased exponent of the largest normal.
    const EMAX: i32 = Self::BIAS;
    /// Bit position of the sign bit.
    const TOP: u32 = Self::E + Self::M;
    /// Exponent field with all bits set (infinities and NaNs).
    const EXP_MASK: u64 = ((1u64 << Self::E) - 1) << Self::M;
    /// Fraction field mask.
    const FRAC_MASK: u64 = (1u64 << Self::M) - 1;
    /// Canonical quiet NaN (positive, payload MSB set).
    const QUIET_NAN: u64 = Self::EXP_MASK | (1u64 << (Self::M - 1));
    /// Working-significand threshold: a normalized value has its top bit at M+3
    /// (three extra guard/round/sticky positions below the M+1 significand bits).
    const WORK: u64 = 1u64 << (Self::M + 3);
    /// Carry threshold: a value at or above this has one bit too many and is
    /// right-normalized by one position with sticky folding.
    const CARRY: u64 = 1u64 << (Self::M + 4);
}

/// CEP:WHAT: f32 format parameters (8 exponent bits, 23 fraction bits).
/// CEP:WHY: Instantiates the generic core for single precision.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: zero-sized type.
/// CEP:EVIDENCE: the f32 matrix test against native.
struct F32P;

impl Params for F32P {
    const E: u32 = 8;
    const M: u32 = 23;
}

/// CEP:WHAT: f64 format parameters (11 exponent bits, 52 fraction bits).
/// CEP:WHY: Instantiates the generic core for double precision.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: zero-sized type.
/// CEP:EVIDENCE: the f64 matrix test against native.
struct F64P;

impl Params for F64P {
    const E: u32 = 11;
    const M: u32 = 52;
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Zero,
    Finite,
    Infinite,
    Nan,
}

/// CEP:WHAT: Decoded IEEE 754 operand: sign, unbiased exponent, integer significand, and
///   the value class.
/// CEP:WHY: The shared unpacked form consumed by the four operations.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: for Finite values sig has the implicit bit set for normals and clear for
///   subnormals (whose exponent is pinned to EMIN).
/// CEP:COST: POD.
/// CEP:EVIDENCE: the native matrix tests.
#[derive(Clone, Copy)]
struct Decoded {
    sign: bool,
    exp: i32,
    sig: u64,
    kind: Kind,
}

/// CEP:WHAT: Splits raw IEEE 754 bits into sign, class, unbiased exponent, and integer
///   significand.
/// CEP:WHY: The single entry into the bit-level representation.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: bits hold a value of the P format in the low E+M+1 bits.
/// CEP:COST: O(1) shifts and masks.
/// CEP:EVIDENCE: the native matrix tests.
fn unpack<P: Params>(bits: u64) -> Decoded {
    let sign = (bits >> P::TOP) & 1 == 1;
    let exp_field = ((bits >> P::M) & ((1u64 << P::E) - 1)) as i32;
    let frac = bits & P::FRAC_MASK;
    if exp_field == ((1i32 << P::E) - 1) {
        if frac == 0 {
            Decoded { sign, exp: P::EMAX, sig: 0, kind: Kind::Infinite }
        } else {
            Decoded { sign, exp: 0, sig: frac, kind: Kind::Nan }
        }
    } else if exp_field == 0 {
        if frac == 0 {
            Decoded { sign, exp: P::EMIN, sig: 0, kind: Kind::Zero }
        } else {
            // Subnormal: no implicit bit, exponent pinned to EMIN.
            Decoded { sign, exp: P::EMIN, sig: frac, kind: Kind::Finite }
        }
    } else {
        Decoded {
            sign,
            exp: exp_field - P::BIAS,
            sig: frac | (1u64 << P::M),
            kind: Kind::Finite,
        }
    }
}

/// CEP:WHAT: Packs a signed zero of the P format.
/// CEP:WHY: Zero results carry a sign bit only.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: the signed-zero unit tests.
fn zero_bits<P: Params>(sign: bool) -> u64 {
    (sign as u64) << P::TOP
}

/// CEP:WHAT: Packs a signed infinity of the P format.
/// CEP:WHY: Overflow and division by zero produce signed infinities.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: the infinity unit tests.
fn inf_bits<P: Params>(sign: bool) -> u64 {
    ((sign as u64) << P::TOP) | P::EXP_MASK
}

/// CEP:WHAT: Normalizes, rounds (round-to-nearest-even), and packs one positive working
///   value `w * 2^(e - M - 3)` plus a sticky flag; the sticky stays a flag through every
///   normalization shift so lost low-order value can never migrate into the significand.
/// CEP:WHY: The single shared exit path of all four operations; keeping one rounding point
///   and one sticky rule is what makes the results bit-identical to hardware.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; overflow past EMAX returns infinity, underflow below EMIN rounds
///   into the subnormal range or to zero.
/// CEP:ASSUMES: w > 0; sticky means "a nonzero amount below one working ulp is already
///   known to be lost" and the caller guarantees nothing larger hides behind it.
/// CEP:COST: O(1) bounded shifts (at most M+4 right steps, M+4 left steps).
/// CEP:EVIDENCE: the rounding-tie and subnormal unit tests and the native matrix.
fn round_pack<P: Params>(sign: bool, mut e: i32, mut w: u64, mut sticky: bool) -> u64 {
    // Right-normalize a one-bit carry, folding the shifted-out bit into sticky.
    while w >= P::CARRY {
        sticky |= (w & 1) != 0;
        w >>= 1;
        e += 1;
    }
    // Left-normalize toward the working width, but never above the EMIN scale. The sticky
    // flag deliberately does not shift: it already means "everything below bit 0".
    while w < P::WORK && e > P::EMIN {
        w <<= 1;
        e -= 1;
    }
    // Below EMIN the result is subnormal-scale: shift right, folding lost bits.
    if e < P::EMIN {
        let shift = (P::EMIN - e) as u32;
        e = P::EMIN;
        if shift > P::M + 3 {
            // Every bit is lost; only the sticky fact survives.
            sticky = sticky || w != 0;
            w = 0;
        } else {
            sticky |= (w & ((1u64 << shift) - 1)) != 0;
            w >>= shift;
        }
    }
    let g = (w >> 2) & 1;
    let r = (w >> 1) & 1;
    let s = sticky || (w & 1) != 0;
    let mut sig = w >> 3;
    if g == 1 && (r == 1 || s || (sig & 1) == 1) {
        sig += 1;
        if sig == 1u64 << (P::M + 1) {
            sig >>= 1;
            e += 1;
        }
    }
    if e > P::EMAX {
        return inf_bits::<P>(sign);
    }
    if sig == 0 {
        return zero_bits::<P>(sign);
    }
    if e == P::EMIN && sig < (1u64 << P::M) {
        // Subnormal encoding: exponent field zero, fraction is the significand.
        return ((sign as u64) << P::TOP) | sig;
    }
    ((sign as u64) << P::TOP) | (((e + P::BIAS) as u64) << P::M) | (sig & P::FRAC_MASK)
}

/// CEP:WHAT: Soft-float addition of two raw IEEE 754 operands of format P; subtraction is
///   expressed through this by flipping the second operand's sign bit.
/// CEP:WHY: The alignment sum is computed exactly in u128 (the shift is bounded by M+3),
///   so guard, round, and sticky bits are exact and rounding matches hardware.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; total per the module contract.
/// CEP:ASSUMES: shifts greater than M+3 discard the small operand, which contributes less
///   than a quarter ulp and can never change the round-to-nearest decision.
/// CEP:COST: O(1) u128/u64 integer work.
/// CEP:EVIDENCE: the native matrix tests.
fn add_bits<P: Params>(a: u64, b: u64) -> u64 {
    let da = unpack::<P>(a);
    let db = unpack::<P>(b);
    if da.kind == Kind::Nan || db.kind == Kind::Nan {
        return P::QUIET_NAN;
    }
    if da.kind == Kind::Zero && db.kind == Kind::Zero {
        // Round-to-nearest: (+0)+(+0) and (+0)+(-0) are +0, (-0)+(-0) is -0.
        return zero_bits::<P>(da.sign && db.sign);
    }
    if da.kind == Kind::Zero {
        return b;
    }
    if db.kind == Kind::Zero {
        return a;
    }
    if da.kind == Kind::Infinite {
        if db.kind == Kind::Infinite && da.sign != db.sign {
            return P::QUIET_NAN;
        }
        return inf_bits::<P>(da.sign);
    }
    if db.kind == Kind::Infinite {
        return inf_bits::<P>(db.sign);
    }
    // Both finite and nonzero. (exp, sig) order is a strict magnitude order.
    let (big, small) = if (da.exp, da.sig) >= (db.exp, db.sig) {
        (da, db)
    } else {
        (db, da)
    };
    let shift = (big.exp - small.exp) as u32;
    let w: u64;
    let mut sticky = false;
    if da.sign == db.sign {
        if shift > P::M + 3 {
            // The small operand contributes less than a quarter working ulp: the exact
            // sum rounds back to the big operand, so it is dropped entirely.
            w = big.sig << 3;
        } else {
            // Exact aligned sum: at most M+5 significant bits.
            let t: u128 = ((big.sig as u128) << (3 + shift)) + ((small.sig as u128) << 3);
            sticky = (t & ((1u128 << shift) - 1)) != 0;
            w = (t >> shift) as u64;
        }
    } else {
        if shift > P::M + 3 {
            // The small operand cannot cancel or round-affect the big one.
            w = big.sig << 3;
        } else {
            let t: u128 = ((big.sig as u128) << (3 + shift)) - ((small.sig as u128) << 3);
            if t == 0 {
                // Exact cancellation: round-to-nearest yields positive zero.
                return zero_bits::<P>(false);
            }
            sticky = (t & ((1u128 << shift) - 1)) != 0;
            w = (t >> shift) as u64;
        }
    }
    let result_sign = if da.sign == db.sign { da.sign } else { big.sign };
    round_pack::<P>(result_sign, big.exp, w, sticky)
}

/// CEP:WHAT: Soft-float multiplication of two raw IEEE 754 operands of format P.
/// CEP:WHY: The significand product is computed exactly in u128 and normalized once, so
///   the single sticky fold at the end is exact.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; total per the module contract (inf*0 is the canonical NaN).
/// CEP:ASSUMES: none
/// CEP:COST: O(1) u128 multiply and shifts.
/// CEP:EVIDENCE: the native matrix tests.
fn mul_bits<P: Params>(a: u64, b: u64) -> u64 {
    let da = unpack::<P>(a);
    let db = unpack::<P>(b);
    if da.kind == Kind::Nan || db.kind == Kind::Nan {
        return P::QUIET_NAN;
    }
    let sign = da.sign ^ db.sign;
    if (da.kind == Kind::Infinite || db.kind == Kind::Infinite)
        && (da.kind == Kind::Zero || db.kind == Kind::Zero)
    {
        return P::QUIET_NAN;
    }
    if da.kind == Kind::Infinite || db.kind == Kind::Infinite {
        return inf_bits::<P>(sign);
    }
    if da.kind == Kind::Zero || db.kind == Kind::Zero {
        return zero_bits::<P>(sign);
    }
    let p: u128 = (da.sig as u128) * (db.sig as u128);
    let len = 128 - p.leading_zeros();
    let e0 = da.exp + db.exp - (P::M as i32) + 3;
    let (w, sticky, e) = if len > P::M + 4 {
        let rs = len - 1 - (P::M + 3);
        let lost = p & ((1u128 << rs) - 1);
        ((p >> rs) as u64, lost != 0, e0 + rs as i32)
    } else {
        let ls = (P::M + 3) - (len - 1);
        ((p << ls) as u64, false, e0 - ls as i32)
    };
    round_pack::<P>(sign, e, w, sticky)
}

/// CEP:WHAT: Soft-float division of two raw IEEE 754 operands of format P.
/// CEP:WHY: The quotient is computed with three extra exact bits (so its top bit always
///   lands at or above the working position and only right shifts ever occur), the
///   remainder and dropped bits become the sticky, and the value is rounded once.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; total per the module contract (0/0 and inf/inf are the canonical
///   NaN; finite/0 is a signed infinity, mirroring IEEE 754).
/// CEP:ASSUMES: none
/// CEP:COST: O(1) u128 divide.
/// CEP:EVIDENCE: the native matrix tests.
fn div_bits<P: Params>(a: u64, b: u64) -> u64 {
    let da = unpack::<P>(a);
    let db = unpack::<P>(b);
    if da.kind == Kind::Nan || db.kind == Kind::Nan {
        return P::QUIET_NAN;
    }
    let sign = da.sign ^ db.sign;
    if da.kind == Kind::Infinite && db.kind == Kind::Infinite {
        return P::QUIET_NAN;
    }
    if da.kind == Kind::Zero && db.kind == Kind::Zero {
        return P::QUIET_NAN;
    }
    if da.kind == Kind::Infinite || db.kind == Kind::Zero {
        return inf_bits::<P>(sign);
    }
    if da.kind == Kind::Zero || db.kind == Kind::Infinite {
        return zero_bits::<P>(sign);
    }
    // Align the dividend so the quotient's top bit always lands at M+5 or M+6: the
    // normalization to the working position is then a right shift of 2 or 3, and the
    // sticky never has to survive a left shift.
    let len_a = 64 - da.sig.leading_zeros();
    let len_b = 64 - db.sig.leading_zeros();
    let k_adjust = (P::M as i32) + 6 - (len_a as i32 - len_b as i32);
    let num: u128 = (da.sig as u128) << k_adjust;
    let q = num / (db.sig as u128);
    let rem = num % (db.sig as u128);
    let len_q = 128 - q.leading_zeros();
    let drop = len_q - 1 - (P::M + 3);
    let dropped = q & ((1u128 << drop) - 1);
    let w = (q >> drop) as u64;
    let sticky = dropped != 0 || rem != 0;
    let e = da.exp - db.exp - k_adjust + drop as i32 + (P::M as i32) + 3;
    round_pack::<P>(sign, e, w, sticky)
}

/// CEP:WHAT: Soft-float f32 addition: raw bits in, raw bits out.
/// CEP:WHY: The compiler-rt symbol the backend may substitute with an ADDSS-class
///   instruction on FPU targets.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; total per the module contract.
/// CEP:ASSUMES: f32 appears only as the exact bit container (to_bits/from_bits), never in
///   arithmetic.
/// CEP:COST: O(1) integer work.
/// CEP:EVIDENCE: the f32 matrix test against native.
pub fn soft_f32_add(a: f32, b: f32) -> f32 {
    f32::from_bits(add_bits::<F32P>(a.to_bits() as u64, b.to_bits() as u64) as u32)
}

/// CEP:WHAT: Soft-float f32 subtraction, expressed as addition with the second operand's
///   sign bit flipped.
/// CEP:WHY: One alignment-and-rounding core serves both add and sub.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; total per the module contract.
/// CEP:ASSUMES: f32 appears only as the exact bit container.
/// CEP:COST: O(1) integer work.
/// CEP:EVIDENCE: the f32 matrix test against native.
pub fn soft_f32_sub(a: f32, b: f32) -> f32 {
    f32::from_bits(add_bits::<F32P>(a.to_bits() as u64, b.to_bits() as u64 ^ (1 << 31)) as u32)
}

/// CEP:WHAT: Soft-float f32 multiplication.
/// CEP:WHY: The compiler-rt symbol for FPU-less targets.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; total per the module contract.
/// CEP:ASSUMES: f32 appears only as the exact bit container.
/// CEP:COST: O(1) integer work.
/// CEP:EVIDENCE: the f32 matrix test against native.
pub fn soft_f32_mul(a: f32, b: f32) -> f32 {
    f32::from_bits(mul_bits::<F32P>(a.to_bits() as u64, b.to_bits() as u64) as u32)
}

/// CEP:WHAT: Soft-float f32 division.
/// CEP:WHY: The compiler-rt symbol for FPU-less targets.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; total per the module contract.
/// CEP:ASSUMES: f32 appears only as the exact bit container.
/// CEP:COST: O(1) integer work.
/// CEP:EVIDENCE: the f32 matrix test against native.
pub fn soft_f32_div(a: f32, b: f32) -> f32 {
    f32::from_bits(div_bits::<F32P>(a.to_bits() as u64, b.to_bits() as u64) as u32)
}

/// CEP:WHAT: Soft-float f64 addition.
/// CEP:WHY: The compiler-rt symbol the backend may substitute with an ADDSD-class
///   instruction on FPU targets.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; total per the module contract.
/// CEP:ASSUMES: f64 appears only as the exact bit container (to_bits/from_bits), never in
///   arithmetic.
/// CEP:COST: O(1) integer work.
/// CEP:EVIDENCE: the f64 matrix test against native.
pub fn soft_f64_add(a: f64, b: f64) -> f64 {
    f64::from_bits(add_bits::<F64P>(a.to_bits(), b.to_bits()))
}

/// CEP:WHAT: Soft-float f64 subtraction, expressed as addition with the second operand's
///   sign bit flipped.
/// CEP:WHY: One alignment-and-rounding core serves both add and sub.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; total per the module contract.
/// CEP:ASSUMES: f64 appears only as the exact bit container.
/// CEP:COST: O(1) integer work.
/// CEP:EVIDENCE: the f64 matrix test against native.
pub fn soft_f64_sub(a: f64, b: f64) -> f64 {
    f64::from_bits(add_bits::<F64P>(a.to_bits(), b.to_bits() ^ (1u64 << 63)))
}

/// CEP:WHAT: Soft-float f64 multiplication.
/// CEP:WHY: The compiler-rt symbol for FPU-less targets.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; total per the module contract.
/// CEP:ASSUMES: f64 appears only as the exact bit container.
/// CEP:COST: O(1) integer work.
/// CEP:EVIDENCE: the f64 matrix test against native.
pub fn soft_f64_mul(a: f64, b: f64) -> f64 {
    f64::from_bits(mul_bits::<F64P>(a.to_bits(), b.to_bits()))
}

/// CEP:WHAT: Soft-float f64 division.
/// CEP:WHY: The compiler-rt symbol for FPU-less targets.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; total per the module contract.
/// CEP:ASSUMES: f64 appears only as the exact bit container.
/// CEP:COST: O(1) integer work.
/// CEP:EVIDENCE: the f64 matrix test against native.
pub fn soft_f64_div(a: f64, b: f64) -> f64 {
    f64::from_bits(div_bits::<F64P>(a.to_bits(), b.to_bits()))
}

// ---------------------------------------------------------------------------
// Integer <-> float conversions (design.md runtime section 5: the software
// intrinsics surface includes int/float conversions for FPU-less targets).
// Semantics mirror Rust's `as` casts exactly: into the float, round-to-nearest-
// even of the exact integer value (with subnormal/overflow IEEE defaults); into
// the integer, truncation toward zero with saturation at the target bounds and
// NaN -> 0.

/// CEP:WHAT: Converts a signed magnitude into the P format with round-to-nearest-even.
/// CEP:WHY: Shared core of the eight int-to-float entry points; the integer is exact,
///   so the only decisions are normalization and the single rounding point.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; total.
/// CEP:ASSUMES: mag != 0 for the nonzero path (callers route zero through here too and
///   get a signed-positive zero).
/// CEP:COST: O(1) shifts.
/// CEP:EVIDENCE: the native differential conversion tests.
fn int_mag_to_float<P: Params>(sign: bool, mag: u64) -> u64 {
    if mag == 0 {
        return zero_bits::<P>(false);
    }
    let len = 64 - mag.leading_zeros(); // 1..=64
    let e = (len - 1) as i32;
    // Bring the top bit to the working position M+3: left-shift while it is below,
    // right-shift (folding lost bits into sticky) once it is above.
    let (w, sticky);
    if len - 1 <= P::M + 3 {
        w = mag << ((P::M + 3) - (len - 1));
        sticky = false;
    } else {
        let shift = len - 1 - (P::M + 3);
        sticky = (mag & ((1u64 << shift) - 1)) != 0;
        w = mag >> shift;
    }
    round_pack::<P>(sign, e, w, sticky)
}

/// CEP:WHAT: Converts one raw P-format value into an integer with Rust `as` semantics:
///   truncation toward zero, saturation at the signed/unsigned W-bit bounds, NaN -> 0.
/// CEP:WHY: Shared core of the eight float-to-int entry points.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; total.
/// CEP:ASSUMES: w <= 127 (all four shipped targets are 32 or 64).
/// CEP:COST: O(1) shifts.
/// CEP:EVIDENCE: the native differential conversion tests.
fn float_bits_to_int<P: Params>(bits: u64, signed: bool, w: u32) -> i128 {
    // Every path - finite, infinite, and the i128-scale saturation - funnels through
    // this clamp so the narrowed public casts are always in range.
    let clamp = |mag: i128| -> i128 {
        if signed {
            let max = (1i128 << (w - 1)) - 1;
            let min = -(1i128 << (w - 1));
            mag.clamp(min, max)
        } else if mag < 0 {
            0
        } else {
            let max = (1i128 << w) - 1;
            mag.clamp(0, max)
        }
    };
    let d = unpack::<P>(bits);
    match d.kind {
        Kind::Nan | Kind::Zero => return 0,
        Kind::Infinite => {
            return clamp(if d.sign { i128::MIN } else { i128::MAX });
        }
        Kind::Finite => {}
    }
    // |value| = d.sig * 2^(d.exp - M); for subnormals exp is pinned to EMIN and the sig
    // lacks the implicit bit, so the same formula holds. The truncated integer part is
    // zero exactly when |value| < 1, i.e. d.exp < 0 for this decoded form (normals have
    // sig >= 2^M, so value >= 2^d.exp).
    if d.exp < 0 {
        return 0;
    }
    let mag: i128 = if d.exp > 126 {
        // Far beyond every i128 scale: saturate at the extreme, clamped below.
        if d.sign {
            i128::MIN
        } else {
            i128::MAX
        }
    } else {
        let e = d.exp as u32;
        let m: i128 = if e > P::M {
            (d.sig as i128) << (e - P::M)
        } else {
            (d.sig as i128) >> (P::M - e)
        };
        if d.sign {
            -m
        } else {
            m
        }
    };
    clamp(mag)
}

/// CEP:WHAT: Soft i32 -> f32 conversion, bit-exact with `x as f32`.
/// CEP:WHY: compiler-rt conversion symbol for FPU-less targets.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; total.
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: native differential tests.
pub fn soft_i32_to_f32(x: i32) -> f32 {
    f32::from_bits(int_mag_to_float::<F32P>(x < 0, x.unsigned_abs() as u64) as u32)
}

/// CEP:WHAT: Soft i32 -> f64 conversion, bit-exact with `x as f64`.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; total.
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: native differential tests.
pub fn soft_i32_to_f64(x: i32) -> f64 {
    f64::from_bits(int_mag_to_float::<F64P>(x < 0, x.unsigned_abs() as u64))
}

/// CEP:WHAT: Soft u32 -> f32 conversion, bit-exact with `x as f32`.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; total.
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: native differential tests.
pub fn soft_u32_to_f32(x: u32) -> f32 {
    f32::from_bits(int_mag_to_float::<F32P>(false, x as u64) as u32)
}

/// CEP:WHAT: Soft u32 -> f64 conversion, bit-exact with `x as f64`.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; total.
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: native differential tests.
pub fn soft_u32_to_f64(x: u32) -> f64 {
    f64::from_bits(int_mag_to_float::<F64P>(false, x as u64))
}

/// CEP:WHAT: Soft i64 -> f32 conversion, bit-exact with `x as f32`.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; total.
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: native differential tests.
pub fn soft_i64_to_f32(x: i64) -> f32 {
    f32::from_bits(int_mag_to_float::<F32P>(x < 0, x.unsigned_abs()) as u32)
}

/// CEP:WHAT: Soft i64 -> f64 conversion, bit-exact with `x as f64`.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; total.
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: native differential tests.
pub fn soft_i64_to_f64(x: i64) -> f64 {
    f64::from_bits(int_mag_to_float::<F64P>(x < 0, x.unsigned_abs()))
}

/// CEP:WHAT: Soft u64 -> f32 conversion, bit-exact with `x as f32`.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; total.
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: native differential tests.
pub fn soft_u64_to_f32(x: u64) -> f32 {
    f32::from_bits(int_mag_to_float::<F32P>(false, x) as u32)
}

/// CEP:WHAT: Soft u64 -> f64 conversion, bit-exact with `x as f64`.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; total.
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: native differential tests.
pub fn soft_u64_to_f64(x: u64) -> f64 {
    f64::from_bits(int_mag_to_float::<F64P>(false, x))
}

/// CEP:WHAT: Soft f32 -> i32 conversion, bit-exact with `x as i32`.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; total (NaN -> 0, saturation at the bounds).
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: native differential tests.
pub fn soft_f32_to_i32(x: f32) -> i32 {
    float_bits_to_int::<F32P>(x.to_bits() as u64, true, 32) as i32
}

/// CEP:WHAT: Soft f32 -> u32 conversion, bit-exact with `x as u32`.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; total (NaN -> 0, negatives -> 0, saturation at the bounds).
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: native differential tests.
pub fn soft_f32_to_u32(x: f32) -> u32 {
    float_bits_to_int::<F32P>(x.to_bits() as u64, false, 32) as u32
}

/// CEP:WHAT: Soft f32 -> i64 conversion, bit-exact with `x as i64`.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; total.
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: native differential tests.
pub fn soft_f32_to_i64(x: f32) -> i64 {
    float_bits_to_int::<F32P>(x.to_bits() as u64, true, 64) as i64
}

/// CEP:WHAT: Soft f32 -> u64 conversion, bit-exact with `x as u64`.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; total.
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: native differential tests.
pub fn soft_f32_to_u64(x: f32) -> u64 {
    float_bits_to_int::<F32P>(x.to_bits() as u64, false, 64) as u64
}

/// CEP:WHAT: Soft f64 -> i32 conversion, bit-exact with `x as i32`.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; total.
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: native differential tests.
pub fn soft_f64_to_i32(x: f64) -> i32 {
    float_bits_to_int::<F64P>(x.to_bits(), true, 32) as i32
}

/// CEP:WHAT: Soft f64 -> u32 conversion, bit-exact with `x as u32`.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; total.
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: native differential tests.
pub fn soft_f64_to_u32(x: f64) -> u32 {
    float_bits_to_int::<F64P>(x.to_bits(), false, 32) as u32
}

/// CEP:WHAT: Soft f64 -> i64 conversion, bit-exact with `x as i64`.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; total.
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: native differential tests.
pub fn soft_f64_to_i64(x: f64) -> i64 {
    float_bits_to_int::<F64P>(x.to_bits(), true, 64) as i64
}

/// CEP:WHAT: Soft f64 -> u64 conversion, bit-exact with `x as u64`.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; total.
/// CEP:ASSUMES: none
/// CEP:COST: O(1).
/// CEP:EVIDENCE: native differential tests.
pub fn soft_f64_to_u64(x: f64) -> u64 {
    float_bits_to_int::<F64P>(x.to_bits(), false, 64) as u64
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// CEP:WHAT: Soft-f32 bit-exactly matches native on representative operands.
    /// CEP:WHY: The hardware is the arbiter of the soft implementation.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert on mismatch (test-only).
    /// CEP:ASSUMES: the host rounds to nearest even.
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn soft_f32_matches_native_spots() {
        let cases = [
            (1.0f32, 2.0f32),
            (-1.5, 0.25),
            (0.1, 0.2),
            (1.0, 3.0e-8),
            (7.0, 3.0),
            (-8.0, -2.5),
            (1e38, 1e38),
            (1e-38, 1e-38),
        ];
        for &(a, b) in &cases {
            assert_eq!(soft_f32_add(a, b).to_bits(), (a + b).to_bits(), "add {a} {b}");
            assert_eq!(soft_f32_sub(a, b).to_bits(), (a - b).to_bits(), "sub {a} {b}");
            assert_eq!(soft_f32_mul(a, b).to_bits(), (a * b).to_bits(), "mul {a} {b}");
            assert_eq!(soft_f32_div(a, b).to_bits(), (a / b).to_bits(), "div {a} {b}");
        }
    }

    /// CEP:WHAT: Soft-f64 bit-exactly matches native on representative operands.
    /// CEP:WHY: The hardware is the arbiter of the soft implementation.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert on mismatch (test-only).
    /// CEP:ASSUMES: the host rounds to nearest even.
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn soft_f64_matches_native_spots() {
        let cases = [
            (1.0f64, 2.0f64),
            (-1.5, 0.25),
            (0.1, 0.2),
            (1.0, 2.0e-308),
            (7.0, 3.0),
            (-8.0, -2.5),
            (1e300, 1e300),
            (5e-324, 5e-324),
        ];
        for &(a, b) in &cases {
            assert_eq!(soft_f64_add(a, b).to_bits(), (a + b).to_bits(), "add {a} {b}");
            assert_eq!(soft_f64_sub(a, b).to_bits(), (a - b).to_bits(), "sub {a} {b}");
            assert_eq!(soft_f64_mul(a, b).to_bits(), (a * b).to_bits(), "mul {a} {b}");
            assert_eq!(soft_f64_div(a, b).to_bits(), (a / b).to_bits(), "div {a} {b}");
        }
    }

    /// CEP:WHAT: Invalid operations return the canonical quiet NaN; overflow returns
    ///   infinity; division by zero returns signed infinity.
    /// CEP:WHY: IEEE 754 default exception behavior is part of the contract.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert on mismatch (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn soft_float_specials_follow_ieee() {
        let nan32 = f32::from_bits(0x7FC0_0000);
        assert_eq!(soft_f32_add(f32::INFINITY, f32::NEG_INFINITY).to_bits(), nan32.to_bits());
        assert_eq!(soft_f32_mul(f32::INFINITY, 0.0f32).to_bits(), nan32.to_bits());
        assert_eq!(soft_f32_div(0.0f32, 0.0f32).to_bits(), nan32.to_bits());
        assert_eq!(soft_f32_div(1.0f32, 0.0f32).to_bits(), f32::INFINITY.to_bits());
        assert_eq!(soft_f32_div(-1.0f32, 0.0f32).to_bits(), f32::NEG_INFINITY.to_bits());
        assert_eq!(
            soft_f32_add(1e38f32, 1e38f32).to_bits(),
            (1e38f32 + 1e38f32).to_bits()
        );
        assert_eq!(
            soft_f32_add(f32::MAX, f32::MAX).to_bits(),
            f32::INFINITY.to_bits()
        );
        assert!(soft_f32_add(f32::NAN, 1.0).is_nan());
        assert!(soft_f64_mul(f64::NAN, 2.0).is_nan());
        let nan64 = f64::from_bits(0x7FF8_0000_0000_0000);
        assert_eq!(soft_f64_div(f64::INFINITY, f64::INFINITY).to_bits(), nan64.to_bits());
        assert_eq!(soft_f64_add(1e308f64, 1e308f64).to_bits(), f64::INFINITY.to_bits());
    }

    /// CEP:WHAT: Subnormal inputs and underflowed results follow IEEE 754 including the
    ///   signed-zero rule and the tie-to-even boundary.
    /// CEP:WHY: Subnormal handling is the hardest part of a soft-float and is explicitly
    ///   required by the task.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert on mismatch (test-only).
    /// CEP:ASSUMES: the host does not flush subnormals to zero.
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn soft_float_subnormals_and_ties() {
        let min_sub = f32::from_bits(1);
        let max_sub = f32::from_bits(0x007F_FFFF);
        assert_eq!(soft_f32_add(min_sub, min_sub).to_bits(), (min_sub + min_sub).to_bits());
        assert_eq!(soft_f32_mul(min_sub, 0.5f32).to_bits(), (min_sub * 0.5f32).to_bits());
        assert_eq!(soft_f32_add(max_sub, max_sub).to_bits(), (max_sub + max_sub).to_bits());
        assert_eq!(soft_f32_mul(0.0f32, -1.0f32).to_bits(), (-0.0f32).to_bits());
        assert_eq!(soft_f32_add(-0.0f32, -0.0f32).to_bits(), (-0.0f32).to_bits());
        assert_eq!(soft_f32_add(-0.0f32, 0.0f32).to_bits(), 0.0f32.to_bits());
        // Exact tie 1 + 2^-24 rounds to even, i.e. back to 1.0.
        let tie = f32::from_bits(0x3380_0000); // 2^-24
        assert_eq!(soft_f32_add(1.0f32, tie).to_bits(), 1.0f32.to_bits());
        // One step past the tie rounds up.
        let past = f32::from_bits(0x33C0_0000); // 1.5 * 2^-24
        assert_eq!(
            soft_f32_add(1.0f32, past).to_bits(),
            f32::from_bits(0x3F80_0001).to_bits()
        );
        let min_sub64 = f64::from_bits(1);
        assert_eq!(
            soft_f64_add(min_sub64, min_sub64).to_bits(),
            (min_sub64 + min_sub64).to_bits()
        );
        assert_eq!(
            soft_f64_mul(min_sub64, 0.5f64).to_bits(),
            (min_sub64 * 0.5f64).to_bits()
        );
    }
    /// CEP:WHAT: Deterministic LCG sweep: the eight int->float conversions match native
    ///   `as` casts bit-exactly across thousands of values plus the adversarial edges
    ///   (powers of two, +/-1 patterns, i64::MIN, u64::MAX).
    /// CEP:WHY: The hardware is the arbiter of the soft conversion.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert on mismatch (test-only).
    /// CEP:ASSUMES: the host rounds to nearest even.
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn int_to_float_matches_native_sweep() {
        let mut state: u64 = 0x9E3779B97F4A7C15;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let mut mags: Vec<u64> = Vec::new();
        for k in 0..64 {
            mags.push(1u64 << k);
            mags.push((1u64 << k).wrapping_sub(1));
            mags.push((1u64 << k).wrapping_add(1));
        }
        for i in 0..1000u64 {
            mags.push(i);
        }
        for _ in 0..4000 {
            mags.push(next());
        }
        for &m in &mags {
            let i64v = m as i64;
            assert_eq!(soft_i64_to_f64(i64v).to_bits(), (i64v as f64).to_bits(), "i64->f64 {i64v}");
            assert_eq!(soft_i64_to_f32(i64v).to_bits(), (i64v as f32).to_bits(), "i64->f32 {i64v}");
            assert_eq!(soft_u64_to_f64(m).to_bits(), (m as f64).to_bits(), "u64->f64 {m}");
            assert_eq!(soft_u64_to_f32(m).to_bits(), (m as f32).to_bits(), "u64->f32 {m}");
            let i32v = m as i32;
            assert_eq!(soft_i32_to_f64(i32v).to_bits(), (i32v as f64).to_bits(), "i32->f64 {i32v}");
            assert_eq!(soft_i32_to_f32(i32v).to_bits(), (i32v as f32).to_bits(), "i32->f32 {i32v}");
            let u32v = m as u32;
            assert_eq!(soft_u32_to_f64(u32v).to_bits(), (u32v as f64).to_bits(), "u32->f64 {u32v}");
            assert_eq!(soft_u32_to_f32(u32v).to_bits(), (u32v as f32).to_bits(), "u32->f32 {u32v}");
        }
        // The i64::MIN magnitude (2^63) is unreachable via wrapping from positives.
        assert_eq!(soft_i64_to_f64(i64::MIN).to_bits(), (i64::MIN as f64).to_bits());
        assert_eq!(soft_i64_to_f32(i64::MIN).to_bits(), (i64::MIN as f32).to_bits());
        assert_eq!(soft_u64_to_f64(u64::MAX).to_bits(), (u64::MAX as f64).to_bits());
        assert_eq!(soft_u64_to_f32(u64::MAX).to_bits(), (u64::MAX as f32).to_bits());
    }

    /// CEP:WHAT: Deterministic LCG sweep: the eight float->int conversions match native
    ///   `as` casts bit-exactly (truncation toward zero, saturation, NaN -> 0) across
    ///   thousands of values plus the adversarial edges (NaN, infinities, subnormals,
    ///   +/-0.5, the exact power-of-two boundaries of every target range).
    /// CEP:WHY: The hardware is the arbiter of the soft conversion.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert on mismatch (test-only).
    /// CEP:ASSUMES: the host implements Rust saturating float->int casts.
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn float_to_int_matches_native_sweep() {
        let nan32 = f32::from_bits(0x7FC0_0000);
        let nan64 = f64::from_bits(0x7FF8_0000_0000_0000);
        let f32_edges = [
            0.0f32,
            -0.0,
            1.0,
            -1.0,
            0.5,
            -0.5,
            1.5,
            -1.5,
            f32::MIN_POSITIVE,
            -f32::MIN_POSITIVE,
            f32::MAX,
            -f32::MAX,
            f32::INFINITY,
            f32::NEG_INFINITY,
            nan32,
            2147483520.0,  // largest f32 below i32::MAX
            2147483648.0,  // i32::MAX + 1 exactly
            -2147483904.0, // smallest f32 magnitude above i32::MIN
            4294967040.0,  // largest f32 below u32::MAX
            4294967296.0,  // u32::MAX + 1 exactly
            9.223372e18,   // near i64::MAX
            1.8446744e19,  // near u64::MAX
        ];
        for &v in &f32_edges {
            assert_eq!(soft_f32_to_i32(v), v as i32, "f32->i32 {v}");
            assert_eq!(soft_f32_to_u32(v), v as u32, "f32->u32 {v}");
            assert_eq!(soft_f32_to_i64(v), v as i64, "f32->i64 {v}");
            assert_eq!(soft_f32_to_u64(v), v as u64, "f32->u64 {v}");
        }
        let f64_edges = [
            0.0f64,
            -0.0,
            1.0,
            -1.0,
            0.5,
            -0.5,
            1.5,
            -1.5,
            f64::MIN_POSITIVE,
            -f64::MIN_POSITIVE,
            f64::MAX,
            -f64::MAX,
            f64::INFINITY,
            f64::NEG_INFINITY,
            nan64,
            2147483647.0,
            2147483648.0,
            -2147483648.0,
            -2147483649.0,
            4294967295.0,
            4294967296.0,
            9223372036854775807.0,
            -9223372036854775808.0,
            18446744073709551615.0,
        ];
        for &v in &f64_edges {
            assert_eq!(soft_f64_to_i32(v), v as i32, "f64->i32 {v}");
            assert_eq!(soft_f64_to_u32(v), v as u32, "f64->u32 {v}");
            assert_eq!(soft_f64_to_i64(v), v as i64, "f64->i64 {v}");
            assert_eq!(soft_f64_to_u64(v), v as u64, "f64->u64 {v}");
        }
        let mut state: u64 = 0xDEADBEEFCAFEBABE;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for _ in 0..4000 {
            let bits32 = next() as u32;
            let v = f32::from_bits(bits32);
            // Skip NaNs whose payload makes the native cast unspecified in either
            // direction (both return 0 here, but keep the sweep robust).
            if !v.is_nan() {
                assert_eq!(soft_f32_to_i32(v), v as i32, "f32->i32 sweep {bits32:08x}");
                assert_eq!(soft_f32_to_u64(v), v as u64, "f32->u64 sweep {bits32:08x}");
            }
            let bits64 = next();
            let w = f64::from_bits(bits64);
            if !w.is_nan() {
                assert_eq!(soft_f64_to_i64(w), w as i64, "f64->i64 sweep {bits64:016x}");
                assert_eq!(soft_f64_to_u32(w), w as u32, "f64->u32 sweep {bits64:016x}");
            }
        }
    }
}
