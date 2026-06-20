use std::hint::black_box;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};

fn format_rows_clone(
    rows: &[(String, (u64, u64, u64))],
) -> Vec<(String, String, String, String, String)> {
    rows.iter()
        .map(|(f, (c, ob, cb))| {
            let r = if *ob > 0 {
                100.0 * ob.saturating_sub(*cb) as f64 / *ob as f64
            } else {
                0.0
            };
            (
                f.clone(),
                format!("{c}x"),
                format!("{ob}"),
                format!("{cb}"),
                format!("{r:.0}%"),
            )
        })
        .collect()
}

fn format_rows_into(
    rows: Vec<(String, (u64, u64, u64))>,
) -> Vec<(String, String, String, String, String)> {
    rows.into_iter()
        .map(|(f, (c, ob, cb))| {
            let r = if ob > 0 {
                100.0 * ob.saturating_sub(cb) as f64 / ob as f64
            } else {
                0.0
            };
            (
                f,
                format!("{c}x"),
                format!("{ob}"),
                format!("{cb}"),
                format!("{r:.0}%"),
            )
        })
        .collect()
}

fn make_rows() -> Vec<(String, (u64, u64, u64))> {
    let mut rows = Vec::new();
    for i in 0..10000 {
        rows.push((
            format!("Filter{i}"),
            (i as u64, (i * 100) as u64, (i * 50) as u64),
        ));
    }
    rows
}

fn criterion_benchmark(c: &mut Criterion) {
    c.bench_function("format_rows_clone", |b| {
        b.iter_batched(
            make_rows,
            |rows| format_rows_clone(black_box(&rows)),
            BatchSize::SmallInput,
        )
    });
    c.bench_function("format_rows_into", |b| {
        b.iter_batched(
            make_rows,
            |rows| format_rows_into(black_box(rows)),
            BatchSize::SmallInput,
        )
    });
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
