//! `.smp` write and read cost as documents and caches grow.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use sm_testkit::large_document;
use std::hint::black_box;
use std::io::Cursor;

/// (features, points per sketch loop) pairs: a small part, a large part,
/// and a part with dense sketches (50k points, near the 8 MiB document limit).
const SIZES: [(usize, usize); 3] = [(10, 4), (1_000, 4), (100, 500)];

fn encode(doc: &sm_document::PartDocument, step: Option<&str>) -> Vec<u8> {
    let mut buf = Cursor::new(Vec::new());
    sm_format::write(&mut buf, doc, step).unwrap();
    buf.into_inner()
}

fn write(c: &mut Criterion) {
    let mut group = c.benchmark_group("smp/write");
    for (features, points) in SIZES {
        let doc = large_document(features, points);
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{features}x{points}")),
            &doc,
            |b, doc| b.iter(|| encode(black_box(doc), None)),
        );
    }
    group.finish();
}

fn read(c: &mut Criterion) {
    let mut group = c.benchmark_group("smp/read");
    for (features, points) in SIZES {
        let bytes = encode(&large_document(features, points), None);
        group.throughput(Throughput::Bytes(bytes.len() as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{features}x{points}")),
            &bytes,
            |b, bytes| b.iter(|| sm_format::read(Cursor::new(black_box(bytes))).unwrap()),
        );
    }
    group.finish();
}

fn read_with_cache(c: &mut Criterion) {
    // The STEP cache is read in full on open (performance review item 4).
    // A synthetic 8 MiB cache shows what that costs on top of the document.
    let doc = large_document(10, 4);
    let step = "#1=CARTESIAN_POINT('',(0.,0.,0.));\n".repeat(8 * 1024 * 1024 / 36);
    let bytes = encode(&doc, Some(&step));
    let mut group = c.benchmark_group("smp/read_with_cache");
    group.sample_size(20);
    group.throughput(Throughput::Bytes(step.len() as u64));
    group.bench_function("8MiB", |b| {
        b.iter(|| sm_format::read(Cursor::new(black_box(&bytes))).unwrap())
    });
    group.finish();
}

fn save(c: &mut Criterion) {
    // The full atomic save path: temp file, fsync, rename, directory fsync.
    let dir = std::env::temp_dir().join(format!("smp-bench-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("part.smp");
    let doc = large_document(1_000, 4);
    let mut group = c.benchmark_group("smp/save");
    group.sample_size(20);
    group.bench_function("1000x4", |b| {
        b.iter(|| sm_format::save(&path, black_box(&doc), None).unwrap())
    });
    group.finish();
    std::fs::remove_dir_all(&dir).unwrap();
}

criterion_group!(benches, write, read, read_with_cache, save);
criterion_main!(benches);
