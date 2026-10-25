//! STEP export cost for rebuilt parts of growing complexity.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use sm_testkit::pocketed_plate;
use std::hint::black_box;

fn export(c: &mut Criterion) {
    let mut group = c.benchmark_group("step/export/pocketed_plate");
    group.sample_size(10);
    for pockets in [1, 10, 50] {
        let model = sm_kernel::rebuild(&pocketed_plate(pockets)).unwrap();
        group.bench_with_input(BenchmarkId::from_parameter(pockets), &model, |b, model| {
            b.iter(|| sm_step::export(black_box(model), "plate.step"))
        });
    }
    group.finish();
}

criterion_group!(benches, export);
criterion_main!(benches);
