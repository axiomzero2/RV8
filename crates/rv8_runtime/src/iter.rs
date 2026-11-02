// CEP:FILE: crates/rv8_runtime/src/iter.rs
// CEP:WHAT: The runtime iterator vocabulary: the Iterator trait (next plus default map,
//   filter, fold, count, and collect_vec_in adapters), the Map and Filter wrapper structs,
//   the Range integer iterator, and the slice Iter.
// CEP:WHY: design.md defines the iterator pattern as central: next returns Option, the
//   adapters are wrapper structs over the wrapped iterator that allocate nothing, and basic
//   implementations exist for ranges and slices.
// CEP:CLASS: HPC-1
// CEP:STATUS: complete
// CEP:FAILURE: none; iteration is infallible, and collect_vec_in surfaces only the Vec
//   growth errors.
// CEP:ASSUMES: the design's zero-sized-adapter claim is realized in the compiler lowering;
//   in this Rust-hosted form the adapters are plain structs holding the wrapped iterator
//   and the closure, allocating nothing.
// CEP:COST: map, filter, fold, and count are O(n) with no allocation; collect_vec_in is
//   O(n) plus amortized growth.
// CEP:EVIDENCE: unit tests in this module.

use crate::allocator::Allocator;
use crate::vec::{CollectionError, Vec};

/// CEP:WHAT: The runtime Iterator trait: one next method plus default adapters implemented
///   in terms of next.
/// CEP:WHY: design.md pins the trait shape and the adapter list.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: iterators are consumed once; Item is Sized.
/// CEP:COST: adapters are O(n), allocation-free.
/// CEP:EVIDENCE: unit tests in this module.
pub trait Iterator {
    type Item;

    /// CEP:WHAT: Yields the next item, or None when exhausted.
    /// CEP:WHY: The single required method per design.md.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: implementation-defined.
    /// CEP:EVIDENCE: this module's tests.
    fn next(&mut self) -> Option<Self::Item>;

    /// CEP:WHAT: Wraps this iterator in a Map adapter applying f to every item.
    /// CEP:WHY: design.md lists map as a default adapter.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(n), no allocation.
    /// CEP:EVIDENCE: test iterator_adapters_over_range.
    fn map<B, F: FnMut(Self::Item) -> B>(self, f: F) -> Map<Self, F>
    where
        Self: Sized,
    {
        Map { iter: self, f }
    }

    /// CEP:WHAT: Wraps this iterator in a Filter adapter keeping items where predicate
    ///   holds.
    /// CEP:WHY: design.md lists filter as a default adapter.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(n), no allocation.
    /// CEP:EVIDENCE: test iterator_adapters_over_range.
    fn filter<P: FnMut(&Self::Item) -> bool>(self, predicate: P) -> Filter<Self, P>
    where
        Self: Sized,
    {
        Filter {
            iter: self,
            predicate,
        }
    }

    /// CEP:WHAT: Folds the iterator into one accumulated value.
    /// CEP:WHY: design.md lists fold as a default method in terms of next.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(n).
    /// CEP:EVIDENCE: test iterator_adapters_over_range.
    fn fold<B, F: FnMut(B, Self::Item) -> B>(mut self, init: B, mut f: F) -> B
    where
        Self: Sized,
    {
        let mut acc = init;
        while let Some(item) = self.next() {
            acc = f(acc, item);
        }
        acc
    }

    /// CEP:WHAT: Counts the remaining items.
    /// CEP:WHY: A minimal extra default method proving the next-based shape.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(n).
    /// CEP:EVIDENCE: test range_iterator_yields_sequence.
    fn count(mut self) -> usize
    where
        Self: Sized,
    {
        let mut total = 0usize;
        while self.next().is_some() {
            total = total.wrapping_add(1);
        }
        total
    }

    /// CEP:WHAT: Collects the items into a runtime Vec over the given allocator.
    /// CEP:WHY: design.md lists collect as a default method into collections; the runtime
    ///   Vec carries its allocator explicitly.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: CollectionError from Vec growth.
    /// CEP:ASSUMES: none
    /// CEP:COST: O(n) plus amortized growth.
    /// CEP:EVIDENCE: test collect_vec_into_runtime_vec.
    fn collect_vec_in<A: Allocator>(self, alloc: A) -> Result<Vec<Self::Item, A>, CollectionError>
    where
        Self: Sized,
    {
        let mut out = Vec::new_in(alloc);
        let mut source = self;
        while let Some(item) = source.next() {
            out.push(item)?;
        }
        Ok(out)
    }
}

/// CEP:WHAT: The Map adapter: wraps an iterator and a mapping function.
/// CEP:WHY: design.md: adapters are wrapper structs holding the iterator and the function.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(1) per item.
/// CEP:EVIDENCE: test iterator_adapters_over_range.
pub struct Map<I, F> {
    iter: I,
    f: F,
}

impl<I: Iterator, B, F: FnMut(I::Item) -> B> Iterator for Map<I, F> {
    type Item = B;

    /// CEP:WHAT: Yields f of the wrapped iterator's next item.
    /// CEP:WHY: The map contract.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1) per item.
    /// CEP:EVIDENCE: this module's tests.
    fn next(&mut self) -> Option<B> {
        match self.iter.next() {
            Some(item) => Some((self.f)(item)),
            None => None,
        }
    }
}

/// CEP:WHAT: The Filter adapter: wraps an iterator and a predicate.
/// CEP:WHY: design.md: adapters are wrapper structs.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(1) per item.
/// CEP:EVIDENCE: test iterator_adapters_over_range.
pub struct Filter<I, P> {
    iter: I,
    predicate: P,
}

impl<I: Iterator, P: FnMut(&I::Item) -> bool> Iterator for Filter<I, P> {
    type Item = I::Item;

    /// CEP:WHAT: Yields the next item satisfying the predicate.
    /// CEP:WHY: The filter contract.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1) per item.
    /// CEP:EVIDENCE: this module's tests.
    fn next(&mut self) -> Option<I::Item> {
        loop {
            let item = self.iter.next()?;
            if (self.predicate)(&item) {
                return Some(item);
            }
        }
    }
}

/// CEP:WHAT: An integer range iterator yielding start, start+1, .., end-1.
/// CEP:WHY: design.md provides Range as a basic iterator implementation.
/// CEP:STATUS: complete
/// CEP:FAILURE: none; next never overflows (start < end guarantees start + 1 <= end).
/// CEP:ASSUMES: T is one of the integer types instantiated by the macro below.
/// CEP:COST: O(1) per item.
/// CEP:EVIDENCE: test range_iterator_yields_sequence.
pub struct Range<T> {
    pub start: T,
    pub end: T,
}

macro_rules! range_for_int {
    ($($t:ty),+ $(,)?) => {
        $(
            impl Iterator for Range<$t> {
                type Item = $t;

                /// CEP:WHAT: Yields the next integer of the half-open range.
                /// CEP:WHY: The Range contract.
                /// CEP:STATUS: complete
                /// CEP:FAILURE: none
                /// CEP:ASSUMES: none
                /// CEP:COST: O(1).
                /// CEP:EVIDENCE: this module's tests.
                fn next(&mut self) -> Option<$t> {
                    if self.start < self.end {
                        let value = self.start;
                        self.start = match value.checked_add(1) {
                            Some(next) => next,
                            None => return Some(value),
                        };
                        Some(value)
                    } else {
                        None
                    }
                }
            }
        )+
    };
}

range_for_int!(i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize);

/// CEP:WHAT: A slice iterator yielding references to the remaining elements.
/// CEP:WHY: design.md provides slice iteration as a basic implementation.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: O(1) per item.
/// CEP:EVIDENCE: test slice_iter_yields_references.
pub struct Iter<'a, T> {
    remaining: &'a [T],
}

impl<'a, T> Iter<'a, T> {
    /// CEP:WHAT: Creates a slice iterator over the whole slice.
    /// CEP:WHY: The constructor of the basic slice iterator.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: this module's tests.
    pub fn new(slice: &'a [T]) -> Iter<'a, T> {
        Iter { remaining: slice }
    }
}

impl<'a, T> Iterator for Iter<'a, T> {
    type Item = &'a T;

    /// CEP:WHAT: Yields a reference to the next element.
    /// CEP:WHY: The slice iterator contract.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: this module's tests.
    fn next(&mut self) -> Option<&'a T> {
        let (first, rest) = self.remaining.split_first()?;
        self.remaining = rest;
        Some(first)
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::bump::Bump;
    use core::ptr::NonNull;

    /// CEP:WHAT: Builds a Bump over a leaked scratch region.
    /// CEP:WHY: Shared fixture.
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

    /// CEP:WHAT: Range yields the exact integer sequence and terminates.
    /// CEP:WHY: The basic range iterator contract.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn range_iterator_yields_sequence() {
        let mut iter = Range {
            start: 0i32,
            end: 5,
        };
        let mut seen = std::vec::Vec::new();
        while let Some(value) = iter.next() {
            seen.push(value);
        }
        assert_eq!(seen, std::vec![0, 1, 2, 3, 4]);
        assert_eq!(iter.next(), None, "exhausted range stays exhausted");
        let empty = Range {
            start: 3u8,
            end: 3,
        };
        assert_eq!(empty.count(), 0);
        let signed = Range {
            start: -2i64,
            end: 2,
        };
        assert_eq!(signed.count(), 4);
    }

    /// CEP:WHAT: map, filter, and fold compose over a range.
    /// CEP:WHY: The default adapter contract.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn iterator_adapters_over_range() {
        let sum_of_squares = Range {
            start: 0u32,
            end: 10,
        }
        .map(|x| x * x)
        .fold(0u32, |acc, x| acc + x);
        assert_eq!(sum_of_squares, 285);
        let evens: std::vec::Vec<u32> = {
            let mut out = std::vec::Vec::new();
            let mut iter = Range {
                start: 0u32,
                end: 10,
            }
            .filter(|x| x % 2 == 0);
            while let Some(value) = iter.next() {
                out.push(value);
            }
            out
        };
        assert_eq!(evens, std::vec![0, 2, 4, 6, 8]);
        let composed: std::vec::Vec<u32> = {
            let mut out = std::vec::Vec::new();
            let mut iter = Range {
                start: 0u32,
                end: 8,
            }
            .filter(|x| x % 2 == 0)
            .map(|x| x + 1);
            while let Some(value) = iter.next() {
                out.push(value);
            }
            out
        };
        assert_eq!(composed, std::vec![1, 3, 5, 7]);
    }

    /// CEP:WHAT: collect_vec_in builds a runtime Vec through the allocator.
    /// CEP:WHY: The collect contract of design.md over the runtime collection.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via expect/assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn collect_vec_into_runtime_vec() {
        let alloc = bump(4096);
        let values: Vec<u32, &Bump> = Range {
            start: 0u32,
            end: 64,
        }
        .map(|x| x * 3)
        .collect_vec_in(&alloc)
        .expect("fits");
        assert_eq!(values.len(), 64);
        assert_eq!(values[0], 0);
        assert_eq!(values[63], 189);
    }

    /// CEP:WHAT: The slice iterator yields references in order.
    /// CEP:WHY: The basic slice iterator contract.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn slice_iter_yields_references() {
        let data = [10u32, 20, 30];
        let mut iter = Iter::new(&data);
        assert_eq!(iter.next(), Some(&10));
        assert_eq!(iter.next(), Some(&20));
        assert_eq!(iter.next(), Some(&30));
        assert_eq!(iter.next(), None);
    }

    /// CEP:WHAT: The adapter structs add no allocation and shrink to their payload.
    /// CEP:WHY: The design's allocation-free adapter claim.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn adapters_hold_iterator_plus_closure() {
        struct ZstIter;
        impl Iterator for ZstIter {
            type Item = u32;

            fn next(&mut self) -> Option<u32> {
                None
            }
        }
        let map = ZstIter.map(|x: u32| x + 1);
        assert_eq!(core::mem::size_of_val(&map), 0, "ZST iterator plus fn-item closure");
        let filtered = ZstIter.filter(|_x: &u32| true);
        assert_eq!(core::mem::size_of_val(&filtered), 0);
    }
}
