// CEP:FILE: crates/rv8_runtime/src/traits.rs
// CEP:WHAT: The runtime trait vocabulary: marker traits (Copy, Clone, Sized, Send, Sync)
//   and the operator traits (Add, Sub, Mul, Div, Rem, Neg, Not, BitAnd, BitOr, BitXor, Shl,
//   Shr) plus the Drop re-export, in their Rust-hosted form.
// CEP:WHY: design.md defines these as the traits every RV8 type implements and that the
//   compiler checks during inference and borrow checking; in the Rust-hosted runtime they
//   are the core definitions, and the compiler lowers operator-trait calls on primitives to
//   machine instructions, bypassing any function call.
// CEP:CLASS: HPC-1
// CEP:STATUS: complete
// CEP:FAILURE: none; re-exports carry the core contracts verbatim.
// CEP:ASSUMES: the RV8 compiler recognizes these trait names and lowers primitive operator
//   implementations to intrinsics; user code never links a call through them on primitives.
// CEP:COST: zero at runtime; traits are compile-time facts.
// CEP:EVIDENCE: unit tests rv8_runtime::traits::tests.

pub use core::clone::Clone;
pub use core::marker::{Copy, Send, Sized, Sync};
pub use core::ops::{
    Add, BitAnd, BitOr, BitXor, Div, Drop, Mul, Neg, Not, Rem, Shl, Shr, Sub,
};

#[cfg(test)]
pub mod tests {
    use super::*;

    /// CEP:WHAT: Marker traits are re-exported and usable as bounds.
    /// CEP:WHY: Public API surface check.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn marker_traits_are_available() {
        fn assert_copy<T: Copy>(value: T) -> T {
            value
        }
        fn assert_send<T: Send>(value: T) -> T {
            value
        }
        fn assert_sync<T: Sync>(value: &T) -> &T {
            value
        }
        assert_eq!(assert_copy(7u32), 7);
        assert_eq!(assert_send(7u32), 7);
        assert_eq!(*assert_sync(&7u32), 7);
        assert_eq!(core::mem::size_of::<u32>(), 4);
    }

    /// CEP:WHAT: The operator traits resolve on primitives and lower to direct arithmetic.
    /// CEP:WHY: The compiler lowers these trait impls to machine instructions.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn operator_traits_lower_on_primitives() {
        fn add<T: Add<Output = T>>(a: T, b: T) -> T {
            a + b
        }
        assert_eq!(add(2u32, 3u32), 5);
        assert_eq!(add(2i64, -3i64), -1);
        let shifted: u32 = 1 << 4;
        assert_eq!(shifted, 16);
        assert_eq!(core::ops::Not::not(0u8), 255);
    }
}
