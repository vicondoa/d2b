use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use criterion::{Criterion, criterion_group, criterion_main};
use redb_resource_store_spike::{
    Mutation, Store, WriteReceipt, fixture_path, put_with_backpressure_retry, synthetic_resource,
};
use tokio::sync::mpsc;

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
    hint_receiver: mpsc::Receiver<redb_resource_store_spike::Hint>,
    background_tasks: Vec<tokio::task::JoinHandle<()>>,
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
        let mut background_tasks = Vec::new();
        if profile.writers > 0 {
            let per_writer_rate = profile.combined_rate / u64::try_from(profile.writers).unwrap();
            let period = Duration::from_micros(1_000_000 / per_writer_rate);
            for writer in 0..profile.writers {
                let background_store = store.clone();
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
                    let mut interval = tokio::time::interval(period);
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
                    }
                }));
            }
        }

        let mut measured = synthetic_resource(1_000_000);
        measured.key.resource_type = "Measured".to_owned();
        measured.key.name = "latency-target".to_owned();
        measured.uid = "latency-target-uid".to_owned();
        measured.owner_uid = None;
        measured.producer_uid = None;
        let receipt = put_with_backpressure_retry(&store, "measured", Mutation::create(measured))
            .await
            .unwrap();
        hint_receiver.recv().await.unwrap();
        Self {
            path,
            store,
            hint_receiver,
            background_tasks,
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
        let hint = self.hint_receiver.recv().await.unwrap();
        assert_eq!(hint.operation_id, operation_id);
        let received_at = Instant::now();
        received_at
            .saturating_duration_since(hint.committed_at)
            .as_secs_f64()
            * 1_000_000.0
    }

    async fn finish(self) {
        for task in self.background_tasks {
            task.abort();
        }
        let stats = self.store.stats().await.unwrap();
        assert_eq!(stats.hint_delivery_failures, 0);
        drop(self.store);
        let _ = std::fs::remove_file(self.path);
    }
}

async fn run_profile(fixture: &mut BenchFixture) -> Vec<f64> {
    let mut samples = Vec::with_capacity(1_000);
    for _ in 0..1_000 {
        samples.push(fixture.sample().await);
    }
    samples
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
        let mut samples = runtime.block_on(run_profile(&mut fixture));
        let p50 = percentile(&mut samples, 50.0);
        let p95 = percentile(&mut samples, 95.0);
        let p99 = percentile(&mut samples, 99.0);
        println!(
            "commit_to_handler profile={} samples=1000 p50_us={p50:.3} p95_us={p95:.3} p99_us={p99:.3}",
            profile.name
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
