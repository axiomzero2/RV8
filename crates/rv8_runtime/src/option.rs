// CEP:FILE: crates/rv8_runtime/src/option.rs
// CEP:WHAT: The Option and Result vocabulary of the runtime, re-exported from core with
//   their contracts pinned: Option<T> is None or Some(T), Result<T, E> is Ok(T) or Err(E).
// CEP:WHY: design.md pins the semantics: is_some/is_none check the variant, unwrap extracts
//   the value and PANICS on None (respectively Err), and map transforms the contained value;
//   the compiler applies the null-pointer optimization to Option<Pointer>-shaped types so
//   the discriminant disappears in memory.
// CEP:CLASS: HPC-1
// CEP:STATUS: complete
// CEP:FAILURE: none; the re-exports carry the core contracts verbatim.
// CEP:ASSUMES: the compiler lowers Option discriminant tests to flag checks and recognizes
//   unwrap as an aggressive-inline candidate per design.md.
// CEP:COST: zero at runtime; the types are compiler primitives.
// CEP:EVIDENCE: unit tests in this module.

pub use core::option::Option;
pub use core::result::Result;

#[cfg(test)]
pub mod tests {
    use super::{Option, Result};

    /// CEP:WHAT: Option variant checks, extraction, and transformation.
    /// CEP:WHY: design.md pins is_some, is_none, unwrap (panic on None), and map.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert and the should_panic case (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn option_semantics_match_design() {
        let some: Option<u32> = Option::Some(7);
        let none: Option<u32> = Option::None;
        assert!(Option::is_some(&some) && !Option::is_none(&some));
        assert!(!Option::is_some(&none) && Option::is_none(&none));
        assert_eq!(Option::unwrap(some), 7);
        assert_eq!(Option::map(some, |v| v * 2), Option::Some(14));
        assert_eq!(Option::map(none, |v: u32| v * 2), Option::None);
    }

    /// CEP:WHAT: Option::unwrap on None panics, per the documented std-mirroring semantics.
    /// CEP:WHY: The design says unwrap panics when None.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics by design (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    #[should_panic]
    fn option_unwrap_none_panics() {
        let none: Option<u32> = Option::None;
        let _ = Option::unwrap(none);
    }

    /// CEP:WHAT: Result variant checks, extraction, and both maps.
    /// CEP:WHY: design.md pins is_ok, is_err, unwrap (panic on Err), map, and map_err.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert and the should_panic case (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn result_semantics_match_design() {
        let ok: Result<u32, &str> = Result::Ok(3);
        let err: Result<u32, &str> = Result::Err("bad");
        assert!(Result::is_ok(&ok) && !Result::is_err(&ok));
        assert!(Result::is_err(&err));
        assert_eq!(Result::unwrap(ok), 3);
        assert_eq!(Result::map(ok, |v| v + 1), Result::Ok(4));
        assert_eq!(Result::map_err(err, |e| e.len()), Result::Err(3));
    }

    /// CEP:WHAT: Result::unwrap on Err panics, per the documented std-mirroring semantics.
    /// CEP:WHY: The design says unwrap panics when Err.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics by design (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    #[should_panic]
    fn result_unwrap_err_panics() {
        let err: Result<u32, &str> = Result::Err("bad");
        let _ = Result::unwrap(err);
    }
}
