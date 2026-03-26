use std::array;
use std::hint::spin_loop;
use std::ptr;
use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};
use std::thread;

pub type Ring<T, const N: usize> = RingCore<T, N, false>;
pub type RingSwap<T, const N: usize> = RingCore<T, N, true>;
pub type RingLoadStore<T, const N: usize> = RingCore<T, N, false>;

#[repr(align(128))]
struct CachePadded<T>(T);

impl<T> CachePadded<T> {
    const fn new(value: T) -> Self {
        Self(value)
    }
}

struct RingSlot<T: Send + Sync> {
    value: AtomicPtr<T>,
    seq: AtomicUsize,
}

pub struct RingCore<T: Send + Sync, const N: usize, const USE_SWAP_ON_TAKE: bool = true> {
    slots: [RingSlot<T>; N],
    enqueue_counter: CachePadded<AtomicUsize>,
    dequeue_counter: CachePadded<AtomicUsize>,
}

impl<T: Send + Sync, const N: usize, const USE_SWAP_ON_TAKE: bool>
    RingCore<T, N, USE_SWAP_ON_TAKE>
{
    pub fn new() -> Self {
        assert!(N > 0, "ring capacity must be greater than zero");

        let slots = array::from_fn(|i| RingSlot {
            value: AtomicPtr::new(ptr::null_mut()),
            seq: AtomicUsize::new(i),
        });

        Self {
            slots,
            enqueue_counter: CachePadded::new(AtomicUsize::new(0)),
            dequeue_counter: CachePadded::new(AtomicUsize::new(0)),
        }
    }

    #[inline]
    pub fn push(&self, value: T) {
        let pos = self.enqueue_counter.0.fetch_add(1, Ordering::Relaxed);
        let slot = &self.slots[Self::slot_index(pos)];

        Self::wait_for_turn(&slot.seq, pos);

        let raw = Box::into_raw(Box::new(value));
        slot.value.store(raw, Ordering::Relaxed);
        slot.seq.store(pos + 1, Ordering::Release);
    }

    #[inline]
    pub fn take(&self) -> T {
        let pos = self.dequeue_counter.0.fetch_add(1, Ordering::Relaxed);
        let slot = &self.slots[Self::slot_index(pos)];

        Self::wait_for_turn(&slot.seq, pos + 1);

        let raw = if USE_SWAP_ON_TAKE {
            slot.value.swap(ptr::null_mut(), Ordering::Relaxed)
        } else {
            let ptr = slot.value.load(Ordering::Relaxed);
            slot.value.store(ptr::null_mut(), Ordering::Relaxed);
            ptr
        };

        assert!(
            !raw.is_null(),
            "slot pointer was null after slot sequence signaled readiness"
        );

        let value = unsafe { *Box::from_raw(raw) };
        slot.seq.store(pos + N, Ordering::Release);
        value
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.enqueue_counter.0.load(Ordering::Acquire)
            == self.dequeue_counter.0.load(Ordering::Acquire)
    }

    #[inline]
    fn slot_index(pos: usize) -> usize {
        if N.is_power_of_two() {
            pos & (N - 1)
        } else {
            pos % N
        }
    }

    #[inline]
    fn wait_for_turn(seq: &AtomicUsize, target: usize) {
        let mut attempts = 0_u32;

        loop {
            if seq.load(Ordering::Acquire) == target {
                return;
            }

            Self::backoff(attempts);
            attempts = attempts.saturating_add(1);
        }
    }

    #[inline]
    fn backoff(attempts: u32) {
        let spins = 1_usize << attempts.min(6);
        for _ in 0..spins {
            spin_loop();
        }

        if attempts >= 10 {
            thread::yield_now();
        }
    }
}

impl<T: Send + Sync, const N: usize, const USE_SWAP_ON_TAKE: bool> Default
    for RingCore<T, N, USE_SWAP_ON_TAKE>
{
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn test_ring_basic_take() {
        const N: usize = 4;
        let ring: Ring<u32, N> = Ring::new();

        ring.push(1);
        ring.push(2);
        ring.push(3);
        ring.push(4);

        assert_eq!(ring.take(), 1);
        assert_eq!(ring.take(), 2);
        assert_eq!(ring.take(), 3);
        assert_eq!(ring.take(), 4);

        for _ in 0..100_000 {
            ring.push(55);
            ring.push(66);
            assert_eq!(ring.take(), 55);
            assert_eq!(ring.take(), 66);
        }
    }

    #[test]
    fn test_ring_supports_non_copy_values() {
        let ring: Ring<String, 8> = Ring::new();

        ring.push("alpha".to_string());
        ring.push("beta".to_string());

        assert_eq!(ring.take(), "alpha");
        assert_eq!(ring.take(), "beta");
        assert!(ring.is_empty());
    }

    #[test]
    fn test_ring_stress_1p_1c() {
        run_stress::<1, 1, 1024, true>(20_000);
    }

    #[test]
    fn test_ring_stress_2p_2c() {
        run_stress::<2, 2, 1024, true>(20_000);
    }

    #[test]
    fn test_ring_stress_6p_6c() {
        run_stress::<6, 6, 1024, true>(12_000);
    }

    #[test]
    fn test_ring_load_store_strategy_stress() {
        run_stress::<2, 2, 1024, false>(20_000);
    }

    fn run_stress<
        const PRODUCERS: usize,
        const CONSUMERS: usize,
        const N: usize,
        const USE_SWAP_ON_TAKE: bool,
    >(
        ops_per_producer: usize,
    ) {
        let ring = Arc::new(RingCore::<u64, N, USE_SWAP_ON_TAKE>::new());
        let total_values = PRODUCERS * ops_per_producer;
        let consumer_splits = split_work(total_values, CONSUMERS);

        let mut producer_handles = Vec::with_capacity(PRODUCERS);
        for producer_id in 0..PRODUCERS {
            let ring = Arc::clone(&ring);
            producer_handles.push(std::thread::spawn(move || {
                for sequence in 0..ops_per_producer {
                    ring.push(encode_value(producer_id, sequence));
                }
            }));
        }

        let mut consumer_handles = Vec::with_capacity(CONSUMERS);
        for take_count in consumer_splits {
            let ring = Arc::clone(&ring);
            consumer_handles.push(std::thread::spawn(move || {
                let mut sum = 0_u128;
                let mut xor = 0_u64;

                for _ in 0..take_count {
                    let value = ring.take();
                    sum += value as u128;
                    xor ^= value;
                }

                (take_count, sum, xor)
            }));
        }

        for handle in producer_handles {
            handle.join().unwrap();
        }

        let mut observed_count = 0_usize;
        let mut observed_sum = 0_u128;
        let mut observed_xor = 0_u64;

        for handle in consumer_handles {
            let (count, sum, xor) = handle.join().unwrap();
            observed_count += count;
            observed_sum += sum;
            observed_xor ^= xor;
        }

        let (expected_sum, expected_xor) = expected_fold(PRODUCERS, ops_per_producer);

        assert_eq!(observed_count, total_values);
        assert_eq!(observed_sum, expected_sum);
        assert_eq!(observed_xor, expected_xor);
        assert!(ring.is_empty());
    }

    fn split_work(total: usize, parts: usize) -> Vec<usize> {
        let base = total / parts;
        let remainder = total % parts;

        (0..parts)
            .map(|index| base + usize::from(index < remainder))
            .collect()
    }

    fn encode_value(producer_id: usize, sequence: usize) -> u64 {
        ((producer_id as u64) << 32) | sequence as u64
    }

    fn expected_fold(producers: usize, ops_per_producer: usize) -> (u128, u64) {
        let mut sum = 0_u128;
        let mut xor = 0_u64;

        for producer_id in 0..producers {
            for sequence in 0..ops_per_producer {
                let value = encode_value(producer_id, sequence);
                sum += value as u128;
                xor ^= value;
            }
        }

        (sum, xor)
    }
}
