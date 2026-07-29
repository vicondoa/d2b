use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use criterion::{Criterion, criterion_group, criterion_main};
use redb_resource_store_spike::{
    Mutation, Store, WriteReceipt, fixture_path, put_with_backpressure_retry, synthetic_resource,
};
use tokio::sync::{Barrier, mpsc};

#[derive(Clone, Copy)]
struct Profile {
    name: &'static str,
    writers: usize,
    combined_rate: u64,
}

const PROFILES: [Profile; 3] = [
    Profile {
        name: "none",
        writers: 0,
        combined_rate: 0,
    },
    Profile {
        name: "10-writers-500-wps",
        writers: 10,
        combined_rate: 500,
    },
    Profile {
        name: "100-writers-2000-wps",
        writers: 100,
        combined_rate: 2_000,
    },
];

struct BenchFixture {
    path: PathBuf,
    store: Store,
    latency_receiver: mpsc::Receiver<(String, f64)>,
    handler_thread: std::thread::JoinHandle<()>,
    background_tasks: Vec<tokio::task::JoinHandle<()>>,
    background_commits: Vec<Arc<AtomicU64>>,
    configured_rate: u64,
    receipt: WriteReceipt,
    sequence: u64,
}

impl BenchFixture {
    async fn new(profile: Profile) -> Self {
        let path = fixture_path(&format!("commit-to-handler-{}", profile.name));
        let store = Store::open(&path).await.unwrap();
        let mut hint_receiver = store
            .hint_consumer(BTreeSet::from(["Measured".to_owned()]))
            .await
            .unwrap();
        let (latency_sender, mut latency_receiver) = mpsc::channel(1_024);
        let handler_thread = std::thread::spawn(move || {
            while let Some(hint) = hint_receiver.blocking_recv() {
                let handled_at = Instant::now();
                let latency_us = handled_at
                    .saturating_duration_since(hint.committed_at)
                    .as_secs_f64()
                    * 1_000_000.0;
                if latency_sender
                    .blocking_send((hint.operation_id, latency_us))
                    .is_err()
                {
                    break;
                }
            }
        });

        let mut measured = synthetic_resource(1_000_000);
        measured.key.resource_type = "Measured".to_owned();
        measured.key.name = "latency-target".to_owned();
        measured.uid = "latency-target-uid".to_owned();
        measured.owner_uid = None;
        measured.producer_uid = None;
        let receipt = put_with_backpressure_retry(&store, "measured", Mutation::create(measured))
            .await
            .unwrap();
        let (operation_id, _) = latency_receiver.recv().await.unwrap();
        assert_eq!(operation_id, "create-latency-target-uid");

        let start_barrier = Arc::new(Barrier::new(profile.writers + 1));
        let mut background_tasks = Vec::new();
        let mut background_commits = Vec::new();
        if profile.writers > 0 {
            let per_writer_rate = profile.combined_rate / u64::try_from(profile.writers).unwrap();
            let period = Duration::from_micros(1_000_000 / per_writer_rate);
            for writer in 0..profile.writers {
                let background_store = store.clone();
                let start_barrier = Arc::clone(&start_barrier);
                let committed = Arc::new(AtomicU64::new(0));
                background_commits.push(Arc::clone(&committed));
                background_tasks.push(tokio::spawn(async move {
                    let mut resource = synthetic_resource(20_000 + writer);
                    resource.key.resource_type = "Background".to_owned();
                    let mut receipt = put_with_backpressure_retry(
                        &background_store,
                        &format!("background-{writer}"),
                        Mutation::create(resource),
                    )
                    .await
                    .unwrap();
                    start_barrier.wait().await;
                    let phase = period
                        .mul_f64(writer as f64 / u64::try_from(profile.writers).unwrap() as f64);
                    tokio::time::sleep(phase).await;
                    let mut interval = tokio::time::interval(period);
                    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                    let mut sequence = 0_u64;
                    loop {
                        interval.tick().await;
                        let mut resource = receipt.resource.clone();
                        resource.generation += 1;
                        sequence += 1;
                        receipt = put_with_backpressure_retry(
                            &background_store,
                            &format!("background-{writer}"),
                            Mutation::update(
                                resource,
                                receipt.revision,
                                format!("background-{writer}-{sequence}"),
                            ),
                        )
                        .await
                        .unwrap();
                        committed.fetch_add(1, Ordering::Relaxed);
                    }
                }));
            }
        }
        start_barrier.wait().await;
        Self {
            path,
            store,
            latency_receiver,
            handler_thread,
            background_tasks,
            background_commits,
            configured_rate: profile.combined_rate,
            receipt,
            sequence: 0,
        }
    }

    async fn sample(&mut self) -> f64 {
        let mut resource = self.receipt.resource.clone();
        resource.generation += 1;
        let operation_id = format!("measured-{}", self.sequence);
        self.sequence += 1;
        let expected_revision = self.receipt.revision;
        self.receipt = put_with_backpressure_retry(
            &self.store,
            "measured",
            Mutation::update(resource, expected_revision, operation_id.clone()),
        )
        .await
        .unwrap();
        let (handled_operation_id, latency_us) = self.latency_receiver.recv().await.unwrap();
        assert_eq!(handled_operation_id, operation_id);
        latency_us
    }

    fn background_counts(&self) -> Vec<u64> {
        self.background_commits
            .iter()
            .map(|counter| counter.load(Ordering::Relaxed))
            .collect()
    }

    async fn finish(self) {
        for task in self.background_tasks {
            task.abort();
            let _ = task.await;
        }
        let stats = self.store.stats().await.unwrap();
        assert_eq!(stats.hint_delivery_failures, 0);
        drop(self.store);
        self.handler_thread.join().unwrap();
        let _ = std::fs::remove_file(self.path);
    }
}

struct ProfileRun {
    samples: Vec<f64>,
    elapsed: Duration,
    background_commits: u64,
    active_writers: usize,
    min_writer_commits: u64,
    achieved_rate: f64,
}

async fn run_profile(fixture: &mut BenchFixture) -> ProfileRun {
    let counts_before = fixture.background_counts();
    let started_at = Instant::now();
    let mut samples = Vec::with_capacity(1_000);
    for _ in 0..1_000 {
        samples.push(fixture.sample().await);
    }
    let elapsed = started_at.elapsed();
    let counts_after = fixture.background_counts();
    let per_writer = counts_after
        .iter()
        .zip(&counts_before)
        .map(|(after, before)| after - before)
        .collect::<Vec<_>>();
    let background_commits = per_writer.iter().sum();
    let active_writers = per_writer.iter().filter(|commits| **commits > 0).count();
    let min_writer_commits = per_writer.iter().copied().min().unwrap_or(0);
    let achieved_rate = background_commits as f64 / elapsed.as_secs_f64();
    if fixture.configured_rate > 0 {
        assert!(
            per_writer.iter().all(|commits| *commits > 0),
            "every configured background writer must commit during measurement"
        );
        let configured = fixture.configured_rate as f64;
        assert!(
            (configured * 0.8..=configured * 1.2).contains(&achieved_rate),
            "achieved background rate {achieved_rate:.1} is outside the documented 20% tolerance around {configured:.1}"
        );
    }
    ProfileRun {
        samples,
        elapsed,
        background_commits,
        active_writers,
        min_writer_commits,
        achieved_rate,
    }
}

fn percentile(samples: &mut [f64], percentile: f64) -> f64 {
    samples.sort_by(f64::total_cmp);
    let rank = percentile / 100.0 * (samples.len() - 1) as f64;
    let floor = rank.floor() as usize;
    let ceiling = rank.ceil() as usize;
    let fraction = rank - floor as f64;
    samples[floor] + (samples[ceiling] - samples[floor]) * fraction
}

fn commit_to_handler(criterion: &mut Criterion) {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    for profile in PROFILES {
        let mut fixture = runtime.block_on(BenchFixture::new(profile));
        let mut run = runtime.block_on(run_profile(&mut fixture));
        let p50 = percentile(&mut run.samples, 50.0);
        let p95 = percentile(&mut run.samples, 95.0);
        let p99 = percentile(&mut run.samples, 99.0);
        println!(
            "commit_to_handler profile={} samples=1000 p50_us={p50:.3} p95_us={p95:.3} p99_us={p99:.3} measurement_s={:.3} background_commits={} active_writers={} min_writer_commits={} achieved_wps={:.1} configured_wps={}",
            profile.name,
            run.elapsed.as_secs_f64(),
            run.background_commits,
            run.active_writers,
            run.min_writer_commits,
            run.achieved_rate,
            profile.combined_rate
        );

        criterion.bench_function(&format!("commit_to_handler/{}", profile.name), |bencher| {
            bencher.iter(|| {
                let latency = runtime.block_on(fixture.sample());
                std::hint::black_box(latency)
            });
        });
        runtime.block_on(fixture.finish());
    }
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(1000);
    targets = commit_to_handler
}
criterion_main!(benches);
