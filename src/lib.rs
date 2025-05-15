use std::array;
use std::fmt::Debug;
use std::ptr;
use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};

struct Ring<T: Send + Sync, const N: usize> {
    slots: [RingSlot<T>; N],
    enqueue_counter: AtomicUsize,
    dequeue_counter: AtomicUsize,
    debug_swap_impossible_value: T, // value to swap in to check whether we're re-reading something we shouldn't be because of the read and write head
}

struct RingSlot<T: Send + Sync> {
    value: AtomicPtr<T>,
    seq: AtomicUsize,
}

// TODO: remove debug and after
impl<T: Send + Sync + Debug + Copy + PartialEq, const N: usize> Ring<T, N> {
    pub fn new(nullval: T) -> Self {
        let slots = array::from_fn(|i| RingSlot {
            value: AtomicPtr::new(ptr::null_mut()),
            seq: AtomicUsize::new(i),
        });

        Self {
            slots: slots,
            enqueue_counter: AtomicUsize::new(0),
            dequeue_counter: AtomicUsize::new(0),
            debug_swap_impossible_value: nullval,
        }
    }

    pub fn push(&self, value: T) {
        let pos = self.enqueue_counter.fetch_add(1, Ordering::Relaxed); // can use relaxed because we are the only one who gets this number
        let slot_idx = pos % N;

        loop {
            let seq = self.slots[slot_idx].seq.load(Ordering::Acquire);
            if seq == pos {
                // The slot is empty, we can put in the value
                break;
            }
            // println!("enqueue waiting for {} got {}", pos, seq);
            // A dequeue hasn't happened yet, so we need to wait
        }

        // We now have the slot, we can put in the value
        self.slots[slot_idx]
            .value
            .store(Box::into_raw(Box::new(value)), Ordering::Release);

        self.slots[slot_idx].seq.store(pos + 1, Ordering::Release);
    }

    pub fn take(&self) -> T {
        let pos = self.dequeue_counter.fetch_add(1, Ordering::Relaxed);
        let slot_idx = pos % N;

        loop {
            let seq = self.slots[slot_idx].seq.load(Ordering::Acquire);
            if seq == pos + 1 {
                // The slot has a value ready, we can take it
                break;
            }
            // A enqueue hasn't happened yet, so we need to wait
        }

        let val_ptr = self.slots[slot_idx].value.load(Ordering::Acquire);
        if val_ptr.is_null() {
            panic!(
                "read null value {:?}, did the read head pass the write head? pos={} slot_idx={}",
                self.debug_swap_impossible_value, pos, slot_idx
            );
        }
        let value = unsafe { *Box::from_raw(val_ptr) };

        self.slots[slot_idx]
            .value
            .store(ptr::null_mut(), Ordering::Release); // do we care about this?

        self.slots[slot_idx].seq.store(pos + N, Ordering::Release);

        value
    }
}

// impl<T: Send + Sync, const N: usize> Drop for Ring<T, N> {
//     // TODO: this might not work right? at least on test failure
//     fn drop(&mut self) {
//         // iterate over all items and free them if they are not null pointers
//         for ptr in self.slots.iter_mut() {
//             if !ptr.load(Ordering::Acquire).is_null() {
//                 // Free it by loading it and dropping it
//                 let _value = unsafe { *Box::from_raw(ptr) };
//                 // Note that we're not swapping in a ptr::null_mut in its place, so we're relying on nothing else
//                 // attempting to access this after (might need to verify that nothing can have a reference after drop?)
//             }
//         }
//     }
// }

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    #[test]
    fn test_ring_basic_take() {
        println!("running");
        const N: usize = 4;
        let ring: Ring<u32, N> = Ring::new(999);
        ring.push(1);
        ring.push(2);
        ring.push(3);
        ring.push(4); // Ring capacity is N-1

        assert_eq!(ring.take(), 1);
        assert_eq!(ring.take(), 2);
        assert_eq!(ring.take(), 3);
        assert_eq!(ring.take(), 4);

        for _ in 0..100000 {
            // Write more to wrap around
            ring.push(55);
            ring.push(66);
            assert_eq!(ring.take(), 55);
            assert_eq!(ring.take(), 66);
        }
    }

    // #[test]
    // fn test_ring_basic_take_sc() {
    //     println!("running");
    //     const N: usize = 4;
    //     let ring: Ring<u32, N> = Ring::new(999);
    //     ring.push(1);
    //     ring.push(2);
    //     ring.push(3);
    //     ring.push(4); // Ring capacity is N-1

    //     assert_eq!(ring.take_sc(), Some(1));
    //     assert_eq!(ring.take_sc(), Some(2));
    //     assert_eq!(ring.take_sc(), Some(3));
    //     assert_eq!(ring.take_sc(), None);

    //     // Write more to wrap around
    //     ring.push(10);
    //     ring.push(20);
    //     ring.push(30);

    //     assert_eq!(ring.take(), Some(10));
    //     assert_eq!(ring.take(), Some(20));
    //     assert_eq!(ring.take(), Some(30));
    // }

    const ARRAY_SIZE: usize = 100;
    const PRODUCERS: usize = 6;
    const CONSUMERS: usize = 6;
    const OPS_PER_PRODUCER: usize = 1_000_000;
    const TRIALS: usize = 3;

    fn mean(data: &[f64]) -> f64 {
        data.iter().sum::<f64>() / data.len() as f64
    }

    fn std_dev(data: &[f64], mean: f64) -> f64 {
        let variance = data.iter().map(|v| (*v - mean).powi(2)).sum::<f64>() / data.len() as f64;
        variance.sqrt()
    }

    fn summarize_trials(lockfree: &[std::time::Duration], mutex: &[std::time::Duration]) {
        let lf_times: Vec<f64> = lockfree.iter().map(|d| d.as_secs_f64() * 1000.0).collect();
        let mx_times: Vec<f64> = mutex.iter().map(|d| d.as_secs_f64() * 1000.0).collect();

        let percent_diffs: Vec<f64> = mx_times
            .iter()
            .zip(&lf_times)
            .map(|(m, l)| ((m - l) / m) * 100.0)
            .collect();

        println!(
            "{:<10} {:>20} {:>25} {:>15}",
            "Trial", "Ring (ms)", "Other<T> :/ (ms)", "Diff (%)"
        );
        for (i, ((lf, mx), diff)) in lf_times
            .iter()
            .zip(&mx_times)
            .zip(&percent_diffs)
            .enumerate()
        {
            println!("{:<10} {:>20.3} {:>25.3} {:>14.2}%", i + 1, lf, mx, diff);
        }

        let mean_lf = mean(&lf_times);
        let mean_mx = mean(&mx_times);
        let mean_diff = mean(&percent_diffs);

        let std_lf = std_dev(&lf_times, mean_lf);
        let std_mx = std_dev(&mx_times, mean_mx);
        let std_diff = std_dev(&percent_diffs, mean_diff);

        println!(
            "{:<10} {:>20.3} {:>25.3} {:>14.2}%",
            "Mean", mean_lf, mean_mx, mean_diff
        );
        println!(
            "{:<10} {:>20.3} {:>25.3} {:>14.2}%",
            "Std Dev", std_lf, std_mx, std_diff
        );
        println!();

        if mean_lf < mean_mx {
            println!("🏁 Winner: Ring (faster by {:.2}% on average)", mean_diff);
        } else {
            println!(
                "🏁 Winner: Mutex<VecDequeue<T>> (faster by {:.2}% on average)",
                -mean_diff
            );
        }
    }

    fn run_vec_dequeue_trial(consumers: usize) -> std::time::Duration {
        let vec: Arc<Mutex<VecDeque<usize>>> =
            Arc::new(Mutex::new(VecDeque::with_capacity(ARRAY_SIZE)));
        let mut handles = Vec::new();

        let start = Instant::now();

        for _ in 0..PRODUCERS {
            let vec = Arc::clone(&vec);
            handles.push(std::thread::spawn(move || {
                for i in 0..OPS_PER_PRODUCER {
                    let mut guard = vec.lock().unwrap();
                    guard.push_back(i);
                }
            }));
        }

        for _ in 0..consumers {
            let vec = Arc::clone(&vec);
            handles.push(std::thread::spawn(move || {
                for _ in 0..OPS_PER_PRODUCER {
                    let mut guard = vec.lock().unwrap();
                    guard.pop_front();
                }
            }));
        }

        for handle in handles.into_iter() {
            handle.join().unwrap();
        }

        std::time::Duration::from_secs_f64(start.elapsed().as_secs_f64())
    }

    fn run_flume_trial(consumers: usize) -> std::time::Duration {
        let (tx, rx) = flume::bounded(ARRAY_SIZE);
        let mut handles = Vec::new();
        let start = Instant::now();
        for _ in 0..PRODUCERS {
            let tx = tx.clone();
            handles.push(std::thread::spawn(move || {
                for i in 0..OPS_PER_PRODUCER {
                    tx.send(i).unwrap();
                }
            }));
        }
        for _ in 0..consumers {
            let rx = rx.clone();
            handles.push(std::thread::spawn(move || {
                for _ in 0..OPS_PER_PRODUCER {
                    rx.recv().unwrap();
                }
            }));
        }
        for handle in handles {
            handle.join().unwrap();
        }
        std::time::Duration::from_secs_f64(start.elapsed().as_secs_f64())
    }

    fn run_crossbeam_trial(consumers: usize) -> std::time::Duration {
        let (tx, rx) = crossbeam_channel::bounded(ARRAY_SIZE);
        let mut handles = Vec::new();
        let start = Instant::now();
        for _ in 0..PRODUCERS {
            let tx = tx.clone();
            handles.push(std::thread::spawn(move || {
                for i in 0..OPS_PER_PRODUCER {
                    tx.send(i).unwrap();
                }
            }));
        }
        for _ in 0..consumers {
            let rx = rx.clone();
            handles.push(std::thread::spawn(move || {
                for _ in 0..OPS_PER_PRODUCER {
                    rx.recv().unwrap();
                }
            }));
        }
        for handle in handles {
            handle.join().unwrap();
        }
        std::time::Duration::from_secs_f64(start.elapsed().as_secs_f64())
    }

    fn run_ring_trial() -> std::time::Duration {
        let ring: Arc<Box<Ring<i64, { ARRAY_SIZE + 1 }>>> = Arc::new(Box::new(Ring::new(-1)));
        let mut handles = Vec::new();

        let start = Instant::now();

        for _ in 0..PRODUCERS {
            let ring = Arc::clone(&ring);
            handles.push(std::thread::spawn(move || {
                for i in 0..OPS_PER_PRODUCER {
                    ring.push(i as i64);
                }
            }));
        }

        for ii in 0..CONSUMERS {
            let ring = Arc::clone(&ring);
            handles.push(std::thread::spawn(move || {
                for _ in 0..OPS_PER_PRODUCER {
                    ring.take();
                }
                println!("consumer done {}", ii);
            }));
        }

        for handle in handles.into_iter() {
            handle.join().unwrap();
        }

        std::time::Duration::from_secs_f64(start.elapsed().as_secs_f64())
    }

    #[test]
    fn test_ring_vs_vecdeque() {
        println!("Running {TRIALS} trials of producer-consumer workloads");
        println!("Producers: {PRODUCERS}, Consumers: {CONSUMERS}, Array Size: {ARRAY_SIZE}");
        println!("------------------------------------------------------");

        let mut lockfree_times = Vec::new();
        let mut vec_dequeue_times = Vec::new();

        for _ in 0..TRIALS {
            vec_dequeue_times.push(run_vec_dequeue_trial(CONSUMERS));
        }
        println!("done vec trials");

        for _ in 0..TRIALS {
            lockfree_times.push(run_ring_trial());
        }

        summarize_trials(&lockfree_times, &vec_dequeue_times);
    }

    #[test]
    fn test_ring_vs_flume() {
        println!("Running {TRIALS} trials of bounded-flume producer-consumer workloads");
        println!("Producers: {PRODUCERS}, Consumers: {CONSUMERS}, Buffer Size: {ARRAY_SIZE}");
        println!("------------------------------------------------------");

        let mut lockfree_times = Vec::new();
        let mut flume_times = Vec::new();
        for _ in 0..TRIALS {
            flume_times.push(run_flume_trial(CONSUMERS));
        }
        for _ in 0..TRIALS {
            lockfree_times.push(run_ring_trial());
        }
        summarize_trials(&lockfree_times, &flume_times);
    }

    #[test]
    fn test_ring_vs_crossbeam() {
        println!("Running {TRIALS} trials of bounded-crossbeam producer-consumer workloads");
        println!("Producers: {PRODUCERS}, Consumers: {CONSUMERS}, Buffer Size: {ARRAY_SIZE}");
        println!("------------------------------------------------------");

        let mut lockfree_times = Vec::new();
        let mut cb_times = Vec::new();
        for _ in 0..TRIALS {
            cb_times.push(run_crossbeam_trial(CONSUMERS));
        }
        for _ in 0..TRIALS {
            lockfree_times.push(run_ring_trial());
        }
        summarize_trials(&lockfree_times, &cb_times);
    }

    #[test]
    fn oldtest() {
        const ITER: usize = 10_000;
        let ring: Ring<usize, { ITER + 1 }> = Ring::new(999); // ring capacity ITER
        let ring_sc: Ring<usize, { ITER + 1 }> = Ring::new(999); // ring capacity ITER
        let deque: Arc<Mutex<VecDeque<usize>>> =
            Arc::new(Mutex::new(VecDeque::with_capacity(ITER)));

        let t_ring = {
            let start = Instant::now();
            for i in 0..ITER {
                ring.push(i);
            }
            for _ in 0..ITER {
                ring.take();
            }
            start.elapsed()
        };

        let t_deque = {
            let start = Instant::now();
            for i in 0..ITER {
                deque.lock().unwrap().push_back(i);
            }
            for _ in 0..ITER {
                deque.lock().unwrap().pop_front().unwrap();
            }
            start.elapsed()
        };

        eprintln!("Ring time: {:?}, VecDeque time: {:?}", t_ring, t_deque);
        // Both should process all items without panicking
    }
}
