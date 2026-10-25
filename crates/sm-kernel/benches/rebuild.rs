//! Full-history rebuild cost as the history grows. Each feature is a boolean
//! against the growing body, so this tracks how rebuild time scales with
//! history length (see the step 1 performance review, item 1).

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use sm_testkit::{large_document, pocketed_plate};
use std::hint::black_box;

fn rebuild(c: &mut Criterion) {
    let mut group = c.benchmark_group("rebuild/pocketed_plate");
    group.sample_size(10);
    for pockets in [1, 10, 50] {
        let doc = pocketed_plate(pockets);
        group.bench_with_input(BenchmarkId::from_parameter(pockets), &doc, |b, doc| {
            b.iter(|| sm_kernel::rebuild(black_box(doc)).unwrap())
        });
    }
    group.finish();
}

fn validate(c: &mut Criterion) {
    // Validation runs on every rebuild, so it has to stay linear.
    let mut group = c.benchmark_group("validate/large_document");
    for features in [100, 1_000, 10_000] {
        let doc = large_document(features, 4);
        group.bench_with_input(BenchmarkId::from_parameter(features), &doc, |b, doc| {
            b.iter(|| black_box(doc).validate().unwrap())
        });
    }
    group.finish();
}

criterion_group!(benches, rebuild, validate);
criterion_main!(benches);
