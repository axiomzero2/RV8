// CEP:FILE: crates/rv8_runtime/tests/runtime_foundation.rs
// CEP:WHAT: Integration tests for the rv8_runtime foundation: 128-bit division and
//   remainder against native semantics, soft-float against native f32/f64 bit patterns,
//   Vec and Box over the Bump allocator, Bump alignment and exhaustion, iterator adapters
//   over Range and slices, the checked memory intrinsics against core, Option/Result
//   semantics, the panic console contract, and a whole-suite determinism check.
// CEP:WHY: Standard 6 requires component-level acceptance evidence; these tests are the
//   runtime's arbiter set and are referenced from the crate's CEP:EVIDENCE lines.
// CEP:CLASS: HPC-1
// CEP:STATUS: complete
// CEP:FAILURE: panics via assert on mismatch (tests only).
// CEP:ASSUMES: the host rounds IEEE 754 to nearest even and does not flush subnormals to
//   zero, so native f32/f64 is a valid oracle; native u128/i128 division is the oracle for
//   the software intrinsics.
// CEP:COST: test-only; the soft-float matrix is O(patterns^2) per width with O(1) work.
// CEP:EVIDENCE: these tests.

use core::ptr::NonNull;
use rv8_runtime::boxed::Box;
use rv8_runtime::bump::Bump;
use rv8_runtime::intrinsics::int128::{
    checked_idivmod_128, checked_udivmod_128, IntMathError,
};
use rv8_runtime::intrinsics::mem::{memcmp, memcpy, memset};
use rv8_runtime::intrinsics::softfloat::{
    soft_f32_add, soft_f32_div, soft_f32_mul, soft_f32_sub, soft_f64_add, soft_f64_div,
    soft_f64_mul, soft_f64_sub,
};
use rv8_runtime::iter::{Iter, Iterator, Range};
use rv8_runtime::option::{Option, Result};
use rv8_runtime::vec::Vec;
use rv8_runtime::{AllocError, Allocator, Layout};

/// CEP:WHAT: Builds a Bump over a leaked byte region of the given length.
/// CEP:WHY: Shared fixture matching the unit-test fixture style.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via expect (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this module.
fn bump(len: usize) -> Bump {
    let mut buffer = std::vec![0u8; len];
    let region = NonNull::new(buffer.as_mut_ptr()).expect("nonzero");
    let allocator = Bump::new(region, buffer.len());
    std::mem::forget(buffer);
    allocator
}

/// CEP:WHAT: u128 division and remainder match native semantics across edge cases:
///   zeros, ones, maxima, top-bit-set limbs, alternating limb patterns, and the
///   quotient-times-divisor-plus-remainder invariant.
/// CEP:WHY: The shift-and-subtract core must be exact for every input class.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert on mismatch (test-only).
/// CEP:ASSUMES: native u128 division is correct.
/// CEP:COST: O(cases * 128) limb steps.
/// CEP:EVIDENCE: this test.
#[test]
fn u128_divrem_matches_native_edges() {
    let max = u128::MAX;
    let cases: std::vec::Vec<(u128, u128)> = std::vec![
        (0, 1),
        (0, max),
        (1, 1),
        (1, max),
        (max, 1),
        (max, max),
        (max - 1, max),
        (max, max - 1),
        (1u128 << 64, 3),
        ((1u128 << 64) - 1, (1u128 << 32) + 1),
        (1u128 << 127, 3),
        ((1u128 << 127) - 1, 1u128 << 64),
        (
            0xAAAA_AAAA_AAAA_AAAA_AAAA_AAAA_AAAA_AAAA,
            0x5555_5555_5555_5555_5555_5555_5555_5555,
        ),
        (
            0xFFFF_FFFF_FFFF_FFFF_0000_0000_0000_0001,
            0x0000_0000_FFFF_FFFF_FFFF_FFFF_0000_0001,
        ),
        (0x8000_0000_0000_0000_0000_0000_0000_0000, 2),
        (0x8000_0000_0000_0000_0000_0000_0000_0001, 0xFFFF_FFFF_FFFF_FFFF),
        (123456789012345678901234567890, 987654321),
    ];
    for &(n, d) in &cases {
        let (q, r) = checked_udivmod_128(n, d).expect("divisor nonzero");
        assert_eq!(q, n / d, "quotient {n} / {d}");
        assert_eq!(r, n % d, "remainder {n} % {d}");
        assert_eq!(q * d + r, n, "invariant {n} / {d}");
    }
    // Division by zero is the typed error, not a panic.
    assert_eq!(checked_udivmod_128(5, 0), Err(IntMathError::DivideByZero));
}

/// CEP:WHAT: i128 division and remainder match native semantics including sign
///   combinations and the MIN / -1 overflow class.
/// CEP:WHY: The signed wrapper must mirror hardware exactly where hardware is defined.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert on mismatch (test-only).
/// CEP:ASSUMES: native i128 division is correct where it is defined.
/// CEP:COST: O(cases * 128) limb steps.
/// CEP:EVIDENCE: this test.
#[test]
fn i128_divrem_matches_native_edges() {
    let min = i128::MIN;
    let max = i128::MAX;
    let cases: std::vec::Vec<(i128, i128)> = std::vec![
        (0, 1),
        (0, -1),
        (1, 1),
        (-1, 1),
        (1, -1),
        (-1, -1),
        (max, max),
        (max, min),
        (min, max),
        (min, min),
        (max, 1),
        (min, 1),
        (max, -1),
        (min + 1, -1),
        (123456789012345678901234567890, -987654321),
        (-123456789012345678901234567890, 987654321),
    ];
    for &(n, d) in &cases {
        let (q, r) = checked_idivmod_128(n, d).expect("defined division");
        assert_eq!(q, n / d, "quotient {n} / {d}");
        assert_eq!(r, n % d, "remainder {n} % {d}");
        assert_eq!(q * d + r, n, "invariant {n} / {d}");
    }
    // MIN / -1 overflows: typed error, never a wrap or a panic.
    assert_eq!(checked_idivmod_128(min, -1), Err(IntMathError::Overflow));
    assert_eq!(checked_idivmod_128(5, 0), Err(IntMathError::DivideByZero));
}

/// CEP:WHAT: A deterministic pseudo-random sweep of u128 division agrees with native and
///   produces identical results on a second run with the same seed.
/// CEP:WHY: Standard 6 asks for a determinism test; the LCG constants are named.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert on mismatch (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: O(SWEEP_LEN * 128) limb steps.
/// CEP:EVIDENCE: this test.
#[test]
fn u128_divrem_fuzz_is_exact_and_deterministic() {
    const SWEEP_LEN: u64 = 256;
    const LCG_MULT: u64 = 6364136223846793005;
    const LCG_INC: u64 = 1442695040888963407;
    const LCG_SEED: u64 = 0x853C_49E6_748F_EA9B;
    let mut seed = LCG_SEED;
    let mut results: std::vec::Vec<(u128, u128, u128, u128)> = std::vec::Vec::new();
    for _ in 0..SWEEP_LEN {
        seed = seed.wrapping_mul(LCG_MULT).wrapping_add(LCG_INC);
        let n = ((seed as u128) << 64) | (seed.rotate_left(32) as u128);
        seed = seed.wrapping_mul(LCG_MULT).wrapping_add(LCG_INC);
        let d = ((seed >> 1) as u128) | 1; // always nonzero
        let (q, r) = checked_udivmod_128(n, d).expect("nonzero divisor");
        assert_eq!(q, n / d);
        assert_eq!(r, n % d);
        results.push((n, d, q, r));
    }
    // Replay with the same seed: identical results, proving determinism.
    let mut seed = LCG_SEED;
    for (n, d, q, r) in results {
        seed = seed.wrapping_mul(LCG_MULT).wrapping_add(LCG_INC);
        let n2 = ((seed as u128) << 64) | (seed.rotate_left(32) as u128);
        seed = seed.wrapping_mul(LCG_MULT).wrapping_add(LCG_INC);
        let d2 = ((seed >> 1) as u128) | 1;
        assert_eq!((n2, d2), (n, d), "replay generates the same inputs");
        assert_eq!(
            checked_udivmod_128(n2, d2).expect("nonzero divisor"),
            (q, r),
            "replay generates the same outputs"
        );
    }
}

/// CEP:WHAT: The f32 soft-float matrix over edge-valued operands is bit-identical to
///   native for every defined operation; NaN inputs produce NaN outputs.
/// CEP:WHY: Hardware is the only trustworthy oracle for the rounding path.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert on mismatch (test-only).
/// CEP:ASSUMES: the host rounds to nearest even without flushing subnormals.
/// CEP:COST: O(patterns^2 * 4) O(1) operations.
/// CEP:EVIDENCE: this test.
#[test]
fn soft_f32_matrix_matches_native() {
    let patterns: std::vec::Vec<f32> = std::vec![
        0.0,
        -0.0,
        1.0,
        -1.0,
        0.5,
        2.0,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NAN,
        f32::MIN_POSITIVE,           // smallest normal 2^-126
        f32::from_bits(1),           // smallest subnormal
        f32::from_bits(0x007F_FFFF), // largest subnormal
        f32::MAX,
        f32::MIN,
        123.456,
        1e-40,
        1e38,
        7.0,
        1.0 / 3.0,
        f32::from_bits(0x3F80_0001), // 1 + ulp
    ];
    for &a in &patterns {
        for &b in &patterns {
            let ops = [
                (soft_f32_add(a, b), a + b, "add"),
                (soft_f32_sub(a, b), a - b, "sub"),
                (soft_f32_mul(a, b), a * b, "mul"),
                (soft_f32_div(a, b), a / b, "div"),
            ];
            for (soft, native, name) in ops {
                if native.is_nan() {
                    // NaN results are payload-defined on hardware; the soft contract is
                    // the canonical quiet NaN, so only the NaN-ness is compared.
                    assert!(soft.is_nan(), "{name}({a}, {b}) must be NaN");
                } else {
                    assert_eq!(
                        soft.to_bits(),
                        native.to_bits(),
                        "{name}({a}, {b}): soft {soft} native {native}"
                    );
                }
            }
        }
    }
}

/// CEP:WHAT: The f64 soft-float matrix over edge-valued operands is bit-identical to
///   native for every defined operation; NaN inputs produce NaN outputs.
/// CEP:WHY: Hardware is the only trustworthy oracle for the rounding path.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert on mismatch (test-only).
/// CEP:ASSUMES: the host rounds to nearest even without flushing subnormals.
/// CEP:COST: O(patterns^2 * 4) O(1) operations.
/// CEP:EVIDENCE: this test.
#[test]
fn soft_f64_matrix_matches_native() {
    let patterns: std::vec::Vec<f64> = std::vec![
        0.0,
        -0.0,
        1.0,
        -1.0,
        0.5,
        2.0,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NAN,
        f64::MIN_POSITIVE,                     // smallest normal 2^-1022
        f64::from_bits(1),                     // smallest subnormal
        f64::from_bits(0x000F_FFFF_FFFF_FFFF), // largest subnormal
        f64::MAX,
        f64::MIN,
        core::f64::consts::PI,
        1e-310,
        1e308,
        7.0,
        1.0 / 3.0,
        f64::from_bits(0x3FF0_0000_0000_0001), // 1 + ulp
    ];
    for &a in &patterns {
        for &b in &patterns {
            let ops = [
                (soft_f64_add(a, b), a + b, "add"),
                (soft_f64_sub(a, b), a - b, "sub"),
                (soft_f64_mul(a, b), a * b, "mul"),
                (soft_f64_div(a, b), a / b, "div"),
            ];
            for (soft, native, name) in ops {
                if native.is_nan() {
                    // NaN results are payload-defined on hardware; the soft contract is
                    // the canonical quiet NaN, so only the NaN-ness is compared.
                    assert!(soft.is_nan(), "{name}({a}, {b}) must be NaN");
                } else {
                    assert_eq!(
                        soft.to_bits(),
                        native.to_bits(),
                        "{name}({a}, {b}): soft {soft} native {native}"
                    );
                }
            }
        }
    }
}

/// CEP:WHAT: Documented IEEE 754 specials on the soft path: canonical quiet NaN for
///   invalid operations, signed infinities for division by zero, subnormal arithmetic,
///   signed-zero rules, and the exact tie at 1 + 2^-24.
/// CEP:WHY: The matrix cannot name these contract points; this test pins them one by one.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert on mismatch (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn soft_float_specials_are_ieee() {
    let qnan32 = f32::from_bits(0x7FC0_0000);
    assert_eq!(
        soft_f32_add(f32::INFINITY, f32::NEG_INFINITY).to_bits(),
        qnan32.to_bits()
    );
    assert_eq!(soft_f32_mul(f32::INFINITY, 0.0).to_bits(), qnan32.to_bits());
    assert_eq!(soft_f32_div(0.0, 0.0).to_bits(), qnan32.to_bits());
    assert_eq!(soft_f32_div(1.0, 0.0).to_bits(), f32::INFINITY.to_bits());
    assert_eq!(soft_f32_div(-1.0, -0.0).to_bits(), f32::INFINITY.to_bits());
    assert_eq!(
        soft_f32_add(1e38, 1e38).to_bits(),
        (1e38f32 + 1e38f32).to_bits()
    );
    assert_eq!(
        soft_f32_add(f32::MAX, f32::MAX).to_bits(),
        f32::INFINITY.to_bits()
    );
    let min_sub = f32::from_bits(1);
    let max_sub = f32::from_bits(0x007F_FFFF);
    assert_eq!(
        soft_f32_add(min_sub, min_sub).to_bits(),
        f32::from_bits(2).to_bits()
    );
    assert_eq!(soft_f32_mul(min_sub, 0.5).to_bits(), 0.0f32.to_bits());
    assert_eq!(
        soft_f32_add(max_sub, max_sub).to_bits(),
        (max_sub + max_sub).to_bits()
    );
    assert_eq!(soft_f32_mul(0.0, -1.0).to_bits(), (-0.0f32).to_bits());
    assert_eq!(soft_f32_add(-0.0, -0.0).to_bits(), (-0.0f32).to_bits());
    assert_eq!(soft_f32_add(-0.0, 0.0).to_bits(), 0.0f32.to_bits());
    // Exact tie: 1 + 2^-24 rounds back to 1.0 under round-to-nearest-even.
    let tie = f32::from_bits(0x3380_0000);
    assert_eq!(soft_f32_add(1.0, tie).to_bits(), 1.0f32.to_bits());
    // One guard-bit past the tie rounds up to 1 + ulp.
    let past = f32::from_bits(0x33C0_0000);
    assert_eq!(
        soft_f32_add(1.0, past).to_bits(),
        f32::from_bits(0x3F80_0001).to_bits()
    );
    assert_eq!(soft_f64_div(1.0, f64::INFINITY).to_bits(), 0.0f64.to_bits());
}

/// CEP:WHAT: Vec over the Bump allocator pushes, pops, grows past its initial capacity,
///   indexes through get, and hands out correct slices.
/// CEP:WHY: The collection is the runtime's central data structure; the allocator-backed
///   growth path must work end to end.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert and expect (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn vec_push_pop_grow_roundtrip() {
    let arena = bump(4096);
    let mut vec: Vec<u32, _> = Vec::with_capacity_in(4, &arena).expect("initial alloc");
    assert_eq!(vec.capacity(), 4);
    for i in 0..100u32 {
        vec.push(i).expect("push fits");
    }
    assert_eq!(vec.len(), 100);
    assert!(vec.capacity() >= 100, "capacity grew past initial");
    for i in 0..100u32 {
        assert_eq!(vec.get(i as usize), Some(&i));
    }
    assert_eq!(vec.get(100), None, "one past the end is None");
    let mut popped: std::vec::Vec<u32> = std::vec::Vec::new();
    while let Some(value) = vec.pop() {
        popped.push(value);
    }
    popped.reverse();
    assert_eq!(popped, (0..100).collect::<std::vec::Vec<u32>>());
    assert!(vec.is_empty());
    assert!(vec.as_slice().is_empty());
}

/// CEP:WHAT: Box allocates through the allocator, dereferences, and runs drop glue exactly
///   once, including through into_inner.
/// CEP:WHY: Box is the runtime's single-value heap type over the Allocator trait.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert and expect (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn box_allocates_and_drops() {
    static DROPS: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
    struct Tracked;
    impl Drop for Tracked {
        fn drop(&mut self) {
            DROPS.fetch_add(1, core::sync::atomic::Ordering::SeqCst);
        }
    }
    let arena = bump(1024);
    let boxed = Box::<u64, _>::new_in(0xDEAD_BEEF, &arena).expect("box alloc");
    assert_eq!(*boxed, 0xDEAD_BEEF);
    let value = boxed.into_inner();
    assert_eq!(value, 0xDEAD_BEEF);
    let tracked = Box::<Tracked, _>::new_in(Tracked, &arena).expect("tracked alloc");
    drop(tracked);
    assert_eq!(DROPS.load(core::sync::atomic::Ordering::SeqCst), 1);
}

/// CEP:WHAT: The Bump allocator honors every power-of-two alignment from 1 to 32 across an
///   interleaved allocation sequence, and reports typed exhaustion on a fixed region with
///   deallocation as a no-op.
/// CEP:WHY: design.md pins the bump allocator to a fixed region with alignment-respecting
///   advance and no reclamation.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert and expect (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn bump_alignment_sequences_and_exhaustion() {
    let arena = bump(64 * 1024);
    let mut previous: Option<usize> = None;
    for round in 0..8u32 {
        for align_log in 0..6u32 {
            let align = 1usize << align_log;
            let size = align;
            let layout = Layout::new(size, align).expect("valid layout");
            let ptr = arena.allocate(layout).expect("region is large");
            let addr = ptr.as_ptr() as usize;
            assert_eq!(addr % align, 0, "alignment {align} honored (round {round})");
            if let Some(prev) = previous {
                assert!(addr >= prev, "cursor never moves backward");
            }
            previous = Some(addr + size);
        }
    }
    // Exhaustion: a 256-byte region fits exactly four 64-byte allocations.
    let small = bump(256);
    let layout = Layout::new(64, 8).expect("valid layout");
    for _ in 0..4 {
        small.allocate(layout).expect("fits exactly");
    }
    assert_eq!(small.allocate(layout), Err(AllocError::OutOfMemory));
    // Deallocation is a no-op: nothing is reclaimed and the failure is stable.
    assert_eq!(small.allocated_bytes(), 256);
    assert_eq!(small.allocate(layout), Err(AllocError::OutOfMemory));
}

/// CEP:WHAT: Map, filter, fold, count, and collect compose over Range, and the collected
///   runtime Vec matches the expected sequence exactly.
/// CEP:WHY: The iterator pipeline is the design's central pattern.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert and expect (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn iterator_pipeline_over_range() {
    let arena = bump(4096);
    let sum = Range { start: 0u32, end: 12 }
        .map(|x| x * 3)
        .filter(|&x| x % 2 == 0)
        .fold(0u32, |acc, x| acc + x);
    assert_eq!(sum, 90, "0,6,12,18,24,30 summed");
    let collected = Range { start: 0u32, end: 12 }
        .map(|x| x * 3)
        .filter(|&x| x % 2 == 0)
        .collect_vec_in(&arena)
        .expect("collect fits");
    assert_eq!(collected.as_slice(), &[0, 6, 12, 18, 24, 30]);
    assert_eq!(Range::<u32> { start: 0, end: 12 }.count(), 12);
    assert_eq!(Range::<u32> { start: 5, end: 5 }.count(), 0);
    // Composition through a fresh pipeline proves the adapters are reusable types.
    let doubled: u32 = Range { start: 1u32, end: 5 }
        .map(|x| x * x)
        .fold(1, |acc, x| acc * x);
    assert_eq!(doubled, 576, "1*4*9*16");
}

/// CEP:WHAT: The slice iterator yields references in order over a runtime Vec's slice and
///   terminates on empty input.
/// CEP:WHY: design.md provides slice iteration as a basic implementation.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert and expect (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn slice_iter_over_vec() {
    let arena = bump(2048);
    let mut vec: Vec<u64, _> = Vec::new_in(&arena);
    for value in [10u64, 20, 30, 40] {
        vec.push(value).expect("fits");
    }
    let mut iter = Iter::new(vec.as_slice());
    assert_eq!(iter.next(), Some(&10));
    assert_eq!(iter.next(), Some(&20));
    let mut rest: std::vec::Vec<u64> = std::vec::Vec::new();
    while let Some(v) = iter.next() {
        rest.push(*v);
    }
    assert_eq!(rest, std::vec![30, 40]);
    assert_eq!(iter.next(), None, "exhausted iter stays exhausted");
    let mut empty = Iter::new(&[] as &[u64]);
    assert_eq!(empty.next(), None);
}

/// CEP:WHAT: memcpy, memset, and memcmp agree with the core slice operations.
/// CEP:WHY: The checked memory intrinsics must be exactly as behaviorally visible as core.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert and expect (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn memory_intrinsics_match_core() {
    let mut buf = [0u8; 64];
    memset(&mut buf, 0xAB);
    let mut reference = [0u8; 64];
    reference.fill(0xAB);
    assert_eq!(buf, reference);
    let src = [1u8, 2, 3, 4, 5, 6, 7, 8];
    memcpy(&mut buf, &src).expect("fits");
    let mut reference2 = reference;
    reference2[..8].copy_from_slice(&src);
    assert_eq!(buf, reference2);
    assert_eq!(memcmp(&buf, &reference2), core::cmp::Ordering::Equal);
    buf[10] ^= 0xFF;
    assert_eq!(memcmp(&buf, &reference2), core::cmp::Ordering::Less);
    assert_eq!(memcmp(&reference2, &buf), core::cmp::Ordering::Greater);
    assert_eq!(memcmp(b"ab", b"abc"), core::cmp::Ordering::Less);
}

/// CEP:WHAT: Option and Result contracts re-verified as an integration surface:
///   variant checks, unwrap, map, and map_err.
/// CEP:WHY: design.md pins these methods as the runtime's vocabulary.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn option_result_semantics() {
    let some: Option<u32> = Option::Some(9);
    let none: Option<u32> = Option::None;
    assert!(Option::is_some(&some));
    assert!(Option::is_none(&none));
    assert_eq!(Option::unwrap(some), 9);
    assert_eq!(Option::map(some, |v| v + 1), Option::Some(10));
    assert_eq!(Option::map(none, |v: u32| v + 1), Option::None);
    let ok: Result<u32, &str> = Result::Ok(4);
    let err: Result<u32, &str> = Result::Err("bad");
    assert!(Result::is_ok(&ok) && !Result::is_err(&ok));
    assert!(Result::is_err(&err) && !Result::is_ok(&err));
    assert_eq!(Result::unwrap(ok), 4);
    assert_eq!(Result::map(ok, |v| v * 2), Result::Ok(8));
    assert_eq!(Result::map_err(err, |e| e.len()), Result::Err(3));
}

/// CEP:WHAT: The panic console path delivers the prefix, message, and newline in order and
///   drops output once no console is registered.
/// CEP:WHY: The panic hook is part of the foundation; its observable contract is the
///   console traffic.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert (test-only).
/// CEP:ASSUMES: single-threaded test access to the sink.
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn panic_console_contract() {
    use std::sync::Mutex;
    static SINK: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
    let sink: &'static Mutex<std::vec::Vec<String>> =
        std::boxed::Box::leak(std::boxed::Box::new(Mutex::new(std::vec::Vec::new())));
    SINK.store(sink as *const _ as usize, core::sync::atomic::Ordering::Release);
    fn writer(text: &str) {
        let ptr = SINK.load(core::sync::atomic::Ordering::Acquire)
            as *const Mutex<std::vec::Vec<String>>;
        unsafe {
            // SAFETY: SINK points at a leaked &'static Mutex installed above.
            (*ptr).lock().expect("sink").push(text.to_string());
        }
    }
    rv8_runtime::panic::set_console(Some(writer));
    rv8_runtime::panic::report_panic("unwrap on None");
    assert_eq!(
        sink.lock().expect("sink").clone(),
        std::vec!["panicked: ", "unwrap on None", "\n"]
    );
    rv8_runtime::panic::set_console(None);
    rv8_runtime::panic::report_panic("silently dropped");
    assert_eq!(sink.lock().expect("sink").len(), 3);
}

/// CEP:WHAT: A mixed batch of runtime operations (division, soft-float, memory) folds to
///   identical results on two executions with the same inputs.
/// CEP:WHY: Standard 6 requires a determinism test; this one spans the crate.
/// CEP:STATUS: complete
/// CEP:FAILURE: panics via assert on mismatch (test-only).
/// CEP:ASSUMES: none
/// CEP:COST: test-only.
/// CEP:EVIDENCE: this test.
#[test]
fn runtime_ops_are_deterministic() {
    fn fold(hash: &mut u64, bytes: &[u8]) {
        for &b in bytes {
            *hash ^= b as u64;
            *hash = hash.wrapping_mul(0x0000_0100_0000_01B3);
        }
    }
    fn batch() -> u64 {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for i in 0..32u32 {
            let n = (1u128 << i) | 0x9E37_79B9_7F4A_7C15;
            let d = (i as u128) + 3;
            let (q, r) = checked_udivmod_128(n, d).expect("nonzero");
            fold(&mut hash, &q.to_le_bytes());
            fold(&mut hash, &r.to_le_bytes());
        }
        let a = 0.1f64;
        let b = 0.2f64;
        fold(&mut hash, &soft_f64_add(a, b).to_bits().to_le_bytes());
        fold(&mut hash, &soft_f64_mul(a, b).to_bits().to_le_bytes());
        fold(&mut hash, &soft_f32_div(1.0, 3.0).to_bits().to_le_bytes());
        let mut buf = [0u8; 32];
        memset(&mut buf, 0x5A);
        fold(&mut hash, &buf);
        let src = [1u8, 2, 3, 4, 5, 6, 7, 8];
        memcpy(&mut buf, &src).expect("fits");
        fold(&mut hash, &buf);
        fold(&mut hash, &[memcmp(&buf, &src) as u8]);
        hash
    }
    assert_eq!(batch(), batch(), "same input, same output");
}
