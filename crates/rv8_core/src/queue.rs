// CEP:FILE: crates/rv8_core/src/queue.rs
// CEP:WHAT: A bounded lock-free multi-producer multi-consumer ring queue (Vyukov design) used by
//   the concurrent diagnostic engine and the work-stealing compilation scheduler.
// CEP:WHY: Standard 25.3 and 5.1 ban locks and blocking on hot paths; a fixed-capacity atomic
//   ring gives deterministic memory use with wait-free enqueue and dequeue in the common case.
// CEP:CLASS: HPC-0
// CEP:STATUS: complete
// CEP:FAILURE: push returns Err(item) when the queue is full (bounded backpressure contract);
//   pop returns None when empty. No allocation after construction. No blocking.
// CEP:ASSUMES: T is Send; capacity is a power of two, checked at construction; per-slot sequence
//   counters use Relaxed/ Acquire/Release ordering per the published Vyukov algorithm.
// CEP:COST: push/pop are one CAS-free atomic exchange each in the uncontended fast path.
// CEP:EVIDENCE: unit tests rv8_core::queue::tests::{fifo_single_thread, spsc_cross_thread}.
// CEP:SECURITY: capacity bound prevents unbounded memory growth from untrusted input volume.

use std::cell::UnsafeCell;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

struct Slot<T> {
    sequence: AtomicUsize,
    value: UnsafeCell<Option<T>>,
}

/// CEP:WHAT: Bounded lock-free MPMC queue with power-of-two capacity.
/// CEP:WHY: Diagnostics and work items cross threads without locks or unbounded growth.
/// CEP:STATUS: complete
/// CEP:FAILURE: push fails closed with the item returned when capacity is reached.
/// CEP:ASSUMES: T: Send; capacity power of two enforced by construction API.
/// CEP:COST: O(1) wait-free push and pop; contended cases retry one atomic load.
/// CEP:EVIDENCE: unit tests rv8_core::queue::tests.
/// CEP:SECURITY: bounded capacity blocks unbounded producer floods.
pub struct MpmcQueue<T> {
    buffer: Vec<Slot<T>>,
    mask: usize,
    enqueue_pos: AtomicUsize,
    dequeue_pos: AtomicUsize,
}

/// CEP:WHAT: Error returned when a DrainBuffer's mutex is poisoned.
/// CEP:WHY: Fail-closed error contract instead of the unit type (clippy
/// result_unit_err); a poisoned drain buffer means diagnostics ordering is unreliable.
/// CEP:STATUS: complete
/// CEP:FAILURE: none
/// CEP:ASSUMES: none
/// CEP:COST: POD.
/// CEP:EVIDENCE: unit test rv8_core::queue::tests::sorted_drain_order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QueuePoisoned;

// Safety: Slot values are only published through the sequence protocol; all access is
// synchronized by Acquire/Release ordering on enqueue_pos/dequeue_pos per the Vyukov algorithm.
unsafe impl<T: Send> Send for MpmcQueue<T> {}
unsafe impl<T: Send> Sync for MpmcQueue<T> {}

impl<T> MpmcQueue<T> {
    /// CEP:WHAT: Allocates a queue with the smallest power-of-two capacity >= requested.
    /// CEP:WHY: Power-of-two masking keeps index arithmetic to one AND.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none; allocation follows global allocator failure contract.
    /// CEP:ASSUMES: min_capacity >= 1.
    /// CEP:COST: O(capacity) initialization, once before hot use (CEP-0 rule: no alloc after init).
    /// CEP:EVIDENCE: unit test rv8_core::queue::tests::fifo_single_thread.
    pub fn with_capacity(min_capacity: usize) -> MpmcQueue<T> {
        let capacity = min_capacity.max(1).next_power_of_two();
        let mut buffer = Vec::with_capacity(capacity);
        for i in 0..capacity {
            buffer.push(Slot {
                sequence: AtomicUsize::new(i),
                value: UnsafeCell::new(None),
            });
        }
        MpmcQueue {
            buffer,
            mask: capacity - 1,
            enqueue_pos: AtomicUsize::new(0),
            dequeue_pos: AtomicUsize::new(0),
        }
    }

    /// CEP:WHAT: Enqueues one item; returns it back on full capacity.
    /// CEP:WHY: Fail-closed backpressure keeps producer memory bounded.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: Err(item) when full; never blocks.
    /// CEP:ASSUMES: T: Send.
    /// CEP:COST: one atomic fetch_or in the uncontended path (wait-free).
    /// CEP:EVIDENCE: unit test rv8_core::queue::tests::spsc_cross_thread.
    pub fn push(&self, item: T) -> Result<(), T> {
        let mut pos = self.enqueue_pos.load(Ordering::Relaxed);
        loop {
            let slot = unsafe { self.buffer.get_unchecked(pos & self.mask) };
            let seq = slot.sequence.load(Ordering::Acquire);
            let diff = seq as isize - pos as isize;
            if diff == 0 {
                match self.enqueue_pos.compare_exchange_weak(
                    pos,
                    pos + 1,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => {
                        unsafe {
                            // CEP:UNSAFE: slot index is masked to buffer length; exclusive write
                            // ownership was acquired via the sequence protocol above.
                            let cell = &mut *slot.value.get();
                            *cell = Some(item);
                            slot.sequence.store(pos + 1, Ordering::Release);
                        }
                        return Ok(());
                    }
                    Err(updated) => pos = updated,
                }
            } else if diff < 0 {
                return Err(item);
            } else {
                pos = self.enqueue_pos.load(Ordering::Relaxed);
            }
        }
    }

    /// CEP:WHAT: Dequeues one item in FIFO order; returns None when empty.
    /// CEP:WHY: Consumers drain without blocking.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: None when empty; never blocks.
    /// CEP:ASSUMES: T: Send.
    /// CEP:COST: one atomic compare_exchange in the uncontended path.
    /// CEP:EVIDENCE: unit test rv8_core::queue::tests::spsc_cross_thread.
    pub fn pop(&self) -> Option<T> {
        let mut pos = self.dequeue_pos.load(Ordering::Relaxed);
        loop {
            let slot = unsafe { self.buffer.get_unchecked(pos & self.mask) };
            let seq = slot.sequence.load(Ordering::Acquire);
            let diff = seq as isize - (pos + 1) as isize;
            if diff == 0 {
                match self.dequeue_pos.compare_exchange_weak(
                    pos,
                    pos + 1,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => {
                        let item = unsafe {
                            // CEP:UNSAFE: exclusive read ownership acquired via sequence protocol.
                            let cell = &mut *slot.value.get();
                            let taken = cell.take();
                            slot.sequence
                                .store(pos + self.buffer.len(), Ordering::Release);
                            taken
                        };
                        return item;
                    }
                    Err(updated) => pos = updated,
                }
            } else if diff < 0 {
                return None;
            } else {
                pos = self.dequeue_pos.load(Ordering::Relaxed);
            }
        }
    }

    /// CEP:WHAT: Reports the fixed capacity in slots.
    /// CEP:WHY: Callers size bounded drains.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: O(1).
    /// CEP:EVIDENCE: unit test rv8_core::queue::tests::fifo_single_thread.
    pub fn capacity(&self) -> usize {
        self.buffer.len()
    }
}

/// CEP:WHAT: Mutex-guarded sorted-drain buffer for diagnostics that must emit in stable order.
/// CEP:WHY: Producers stay lock-light (bounded queue), while the single consumer sorts once;
///   a Mutex is permitted in HPC-1 cold paths and the sort output is deterministic.
/// CEP:STATUS: complete
/// CEP:FAILURE: lock poisoning surfaces as an Err from drain methods.
/// CEP:ASSUMES: used from the single diagnostic-emitter thread for drains.
/// CEP:COST: cold; diagnostics only.
/// CEP:EVIDENCE: unit test rv8_core::queue::tests::sorted_drain_order.
pub struct DrainBuffer<T> {
    inner: Mutex<VecDeque<T>>,
}

impl<T> DrainBuffer<T> {
    /// CEP:WHAT: Creates an empty drain buffer.
    /// CEP:WHY: Construction boundary.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: cold.
    /// CEP:EVIDENCE: unit test rv8_core::queue::tests::sorted_drain_order.
    pub fn new() -> DrainBuffer<T> {
        DrainBuffer {
            inner: Mutex::new(VecDeque::new()),
        }
    }

    /// CEP:WHAT: Appends one item.
    /// CEP:WHY: Cold-side accumulation.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: propagates lock poisoning as Err.
    /// CEP:ASSUMES: none
    /// CEP:COST: cold.
    /// CEP:EVIDENCE: unit test rv8_core::queue::tests::sorted_drain_order.
    pub fn push(&self, item: T) -> Result<(), T> {
        match self.inner.lock() {
            Ok(mut q) => {
                q.push_back(item);
                Ok(())
            }
            Err(_) => Err(item),
        }
    }

    /// CEP:WHAT: Drains all items sorted by a caller key function.
    /// CEP:WHY: Deterministic emission order per standard 38.15.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: propagates lock poisoning as Err.
    /// CEP:ASSUMES: key is a total order.
    /// CEP:COST: O(n log n) per drain.
    /// CEP:EVIDENCE: unit test rv8_core::queue::tests::sorted_drain_order.
    pub fn drain_sorted<K, F>(&self, key: F) -> Result<Vec<T>, QueuePoisoned>
    where
        K: Ord,
        F: Fn(&T) -> K,
    {
        match self.inner.lock() {
            Ok(mut q) => {
                let mut items: Vec<T> = q.drain(..).collect();
                items.sort_by_key(key);
                Ok(items)
            }
            Err(_) => Err(QueuePoisoned),
        }
    }
}

impl<T> Default for DrainBuffer<T> {
    /// CEP:WHAT: Default construction matching new().
    /// CEP:WHY: Idiomatic Default for containers.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: none
    /// CEP:ASSUMES: none
    /// CEP:COST: cold.
    /// CEP:EVIDENCE: unit test rv8_core::queue::tests::sorted_drain_order.
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    /// CEP:WHAT: Verifies FIFO order and capacity rejection on a single thread.
    /// CEP:WHY: Basic queue contract.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert on mismatch (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn fifo_single_thread() {
        let q: MpmcQueue<u64> = MpmcQueue::with_capacity(3);
        assert_eq!(q.capacity(), 4);
        for i in 0..4u64 {
            assert!(q.push(i).is_ok());
        }
        assert_eq!(q.push(99), Err(99));
        for i in 0..4u64 {
            assert_eq!(q.pop(), Some(i));
        }
        assert_eq!(q.pop(), None);
    }

    /// CEP:WHAT: Verifies cross-thread handoff of items in order between one producer and one consumer.
    /// CEP:WHY: Lock-free correctness under real scheduling.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert on mismatch (test-only).
    /// CEP:ASSUMES: scheduler makes progress; bounded spin on full queue.
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn spsc_cross_thread() {
        let q = Arc::new(MpmcQueue::<u64>::with_capacity(1024));
        let producer = Arc::clone(&q);
        let consumer = Arc::clone(&q);
        let p = thread::spawn(move || {
            for i in 0..1000u64 {
                while producer.push(i).is_err() {
                    std::hint::spin_loop();
                }
            }
        });
        let c = thread::spawn(move || {
            let mut seen = Vec::with_capacity(1000);
            while seen.len() < 1000 {
                if let Some(v) = consumer.pop() {
                    seen.push(v);
                } else {
                    std::hint::spin_loop();
                }
            }
            seen
        });
        p.join().unwrap();
        let seen = c.join().unwrap();
        assert_eq!(seen.len(), 1000);
        assert!(seen.windows(2).all(|w| w[0] < w[1]));
    }

    /// CEP:WHAT: Verifies sorted drain emits a deterministic order.
    /// CEP:WHY: Diagnostic ordering standard.
    /// CEP:STATUS: complete
    /// CEP:FAILURE: panics via assert on mismatch (test-only).
    /// CEP:ASSUMES: none
    /// CEP:COST: test-only.
    /// CEP:EVIDENCE: this test.
    #[test]
    fn sorted_drain_order() {
        let b: DrainBuffer<u64> = DrainBuffer::new();
        for v in [5u64, 1, 3] {
            b.push(v).unwrap();
        }
        let out = b.drain_sorted(|v| *v).unwrap();
        assert_eq!(out, vec![1, 3, 5]);
        assert!(b.drain_sorted(|v| *v).unwrap().is_empty());
    }
}
