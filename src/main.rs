use crossbeam_channel::{Receiver, Sender};
use rust_ring_buffer::{Ring, RingCore, RingSwap};
use std::env;
use std::sync::Arc;
use std::thread;
use std::time::Instant;

const DEFAULT_RUNS: usize = 5;
const DEFAULT_OPS_PER_PRODUCER: usize = 500_000;
const CAPACITIES: [usize; 2] = [1024, 65_536];
const SCENARIOS: [Scenario; 3] = [
    Scenario::new(1, 1),
    Scenario::new(2, 2),
    Scenario::new(6, 6),
];

trait Queue: Send + Sync + 'static {
    fn push(&self, value: u64);
    fn take(&self) -> u64;
    fn is_empty(&self) -> bool;
}

type DefaultRing<const N: usize> = RingCore<u64, N, false>;
type SwapRing<const N: usize> = RingCore<u64, N, true>;

impl<const N: usize> Queue for DefaultRing<N> {
    fn push(&self, value: u64) {
        RingCore::push(self, value);
    }

    fn take(&self) -> u64 {
        RingCore::take(self)
    }

    fn is_empty(&self) -> bool {
        RingCore::is_empty(self)
    }
}

impl<const N: usize> Queue for SwapRing<N> {
    fn push(&self, value: u64) {
        RingCore::push(self, value);
    }

    fn take(&self) -> u64 {
        RingCore::take(self)
    }

    fn is_empty(&self) -> bool {
        RingCore::is_empty(self)
    }
}

struct CrossbeamQueue {
    tx: Sender<u64>,
    rx: Receiver<u64>,
}

impl CrossbeamQueue {
    fn bounded(capacity: usize) -> Self {
        let (tx, rx) = crossbeam_channel::bounded(capacity);
        Self { tx, rx }
    }
}

impl Queue for CrossbeamQueue {
    fn push(&self, value: u64) {
        self.tx.send(value).unwrap();
    }

    fn take(&self) -> u64 {
        self.rx.recv().unwrap()
    }

    fn is_empty(&self) -> bool {
        self.rx.is_empty()
    }
}

#[derive(Clone, Copy)]
struct Scenario {
    producers: usize,
    consumers: usize,
}

impl Scenario {
    const fn new(producers: usize, consumers: usize) -> Self {
        Self {
            producers,
            consumers,
        }
    }
}

#[derive(Clone, Copy)]
struct Config {
    runs: usize,
    ops_per_producer: usize,
}

#[derive(Clone, Copy)]
struct Sample {
    seconds: f64,
    ops_per_sec: f64,
}

#[derive(Clone, Copy)]
struct Summary {
    median_seconds: f64,
    median_ops_per_sec: f64,
    best_ops_per_sec: f64,
}

fn main() {
    let config = parse_config();

    println!(
        "Benchmarking aggregate push+pop throughput with {} runs and {} ops/producer",
        config.runs, config.ops_per_producer
    );
    println!("Implementations: ring(default), ring(swap), crossbeam");

    for capacity in CAPACITIES {
        println!();
        println!("Capacity {}", capacity);
        println!(
            "{:<10} {:<20} {:>14} {:>14} {:>12}",
            "Scenario", "Implementation", "Median Mops/s", "Best Mops/s", "Median ms"
        );

        for scenario in SCENARIOS {
            match capacity {
                1024 => run_capacity::<1024>(scenario, config),
                65_536 => run_capacity::<65_536>(scenario, config),
                _ => unreachable!("unexpected benchmark capacity"),
            }
        }
    }
}

fn run_capacity<const N: usize>(scenario: Scenario, config: Config) {
    let scenario_label = scenario_label(scenario);

    let ring_swap = benchmark_impl::<SwapRing<N>, _>(scenario, config, || RingSwap::new());
    print_summary(&scenario_label, "ring(swap)", ring_swap);

    let ring_default = benchmark_impl::<DefaultRing<N>, _>(scenario, config, || Ring::new());
    print_summary(&scenario_label, "ring(default)", ring_default);

    let crossbeam =
        benchmark_impl::<CrossbeamQueue, _>(scenario, config, || CrossbeamQueue::bounded(N));
    print_summary(&scenario_label, "crossbeam", crossbeam);
}

fn benchmark_impl<Q, F>(scenario: Scenario, config: Config, build: F) -> Summary
where
    Q: Queue,
    F: Fn() -> Q + Copy,
{
    let mut samples = Vec::with_capacity(config.runs);

    for _ in 0..config.runs {
        samples.push(run_sample(build(), scenario, config.ops_per_producer));
    }

    summarize(&samples)
}

fn run_sample<Q: Queue>(queue: Q, scenario: Scenario, ops_per_producer: usize) -> Sample {
    let queue = Arc::new(queue);
    let total_values = scenario.producers * ops_per_producer;
    let consumer_splits = split_work(total_values, scenario.consumers);
    let mut producer_handles = Vec::with_capacity(scenario.producers);
    let mut consumer_handles = Vec::with_capacity(scenario.consumers);

    let start = Instant::now();

    for producer_id in 0..scenario.producers {
        let queue = Arc::clone(&queue);
        producer_handles.push(thread::spawn(move || {
            for sequence in 0..ops_per_producer {
                queue.push(encode_value(producer_id, sequence));
            }
        }));
    }

    for take_count in consumer_splits {
        let queue = Arc::clone(&queue);
        consumer_handles.push(thread::spawn(move || {
            for _ in 0..take_count {
                std::hint::black_box(queue.take());
            }
            take_count
        }));
    }

    for handle in producer_handles {
        handle.join().unwrap();
    }

    let mut consumed = 0_usize;
    for handle in consumer_handles {
        consumed += handle.join().unwrap();
    }

    let elapsed = start.elapsed().as_secs_f64();
    let total_ops = (total_values * 2) as f64;

    assert_eq!(consumed, total_values, "consumer count mismatch");
    assert!(queue.is_empty(), "queue was not empty after benchmark run");

    Sample {
        seconds: elapsed,
        ops_per_sec: total_ops / elapsed,
    }
}

fn summarize(samples: &[Sample]) -> Summary {
    let mut seconds: Vec<f64> = samples.iter().map(|sample| sample.seconds).collect();
    let mut ops_per_sec: Vec<f64> = samples.iter().map(|sample| sample.ops_per_sec).collect();

    seconds.sort_by(f64::total_cmp);
    ops_per_sec.sort_by(f64::total_cmp);

    Summary {
        median_seconds: seconds[seconds.len() / 2],
        median_ops_per_sec: ops_per_sec[ops_per_sec.len() / 2],
        best_ops_per_sec: ops_per_sec[ops_per_sec.len() - 1],
    }
}

fn print_summary(scenario: &str, implementation: &str, summary: Summary) {
    println!(
        "{:<10} {:<20} {:>14.2} {:>14.2} {:>12.2}",
        scenario,
        implementation,
        summary.median_ops_per_sec / 1_000_000.0,
        summary.best_ops_per_sec / 1_000_000.0,
        summary.median_seconds * 1_000.0
    );
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

fn scenario_label(scenario: Scenario) -> String {
    format!("{}P/{}C", scenario.producers, scenario.consumers)
}

fn parse_config() -> Config {
    let mut config = Config {
        runs: DEFAULT_RUNS,
        ops_per_producer: DEFAULT_OPS_PER_PRODUCER,
    };

    for argument in env::args().skip(1) {
        if let Some(value) = argument.strip_prefix("--runs=") {
            config.runs = value.parse().expect("invalid value for --runs");
        } else if let Some(value) = argument.strip_prefix("--ops=") {
            config.ops_per_producer = value.parse().expect("invalid value for --ops");
        } else {
            panic!("unsupported argument: {argument}");
        }
    }

    assert!(config.runs > 0, "--runs must be greater than zero");
    assert!(
        config.ops_per_producer > 0,
        "--ops must be greater than zero"
    );

    config
}
