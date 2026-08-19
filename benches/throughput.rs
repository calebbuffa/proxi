use criterion::{
    BatchSize, BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main,
};
use proxi::{AngularUnits, Context, ContextOptions, Coord3, Direction, Geod, TransformerBuilder};

fn transformer<'a>(context: &'a Context) -> proxi::Transformer<'a> {
    TransformerBuilder::new(context, "EPSG:4978", "EPSG:32633")
        .build()
        .expect("build benchmark transformer")
}

fn bench_transformer_construction(c: &mut Criterion) {
    let context = Context::configured().expect("context");
    c.bench_function("transformer_construction", |b| {
        b.iter(|| {
            let transformer = TransformerBuilder::new(&context, "EPSG:4978", "EPSG:32633")
                .build()
                .expect("build transformer");
            black_box(transformer);
        });
    });
}

fn bench_scalar_reuse(c: &mut Criterion) {
    let context = Context::configured().expect("context");
    let mut transformer = transformer(&context);
    c.bench_function("scalar_xyz_reuse", |b| {
        b.iter(|| {
            let result = transformer
                .forward_xyz(black_box(Coord3::new(6378137.0, 1000.0, 20.0)))
                .expect("scalar transform");
            black_box(result);
        });
    });
}

fn bench_soa(c: &mut Criterion) {
    let mut group = c.benchmark_group("soa_xyz_roundtrip");
    for size in [64usize, 1024, 16_384] {
        group.throughput(Throughput::Elements(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, &size| {
            let context = Context::configured().expect("context");
            let mut transformer = transformer(&context);
            let mut x = vec![6378137.0; size];
            let mut y = vec![1000.0; size];
            let mut z = vec![20.0; size];
            b.iter(|| {
                transformer
                    .transform_xyz_in_place(
                        black_box(&mut x),
                        black_box(&mut y),
                        black_box(&mut z),
                        Direction::Forward,
                        AngularUnits::Auto,
                    )
                    .expect("soa forward");
                transformer
                    .transform_xyz_in_place(
                        black_box(&mut x),
                        black_box(&mut y),
                        black_box(&mut z),
                        Direction::Inverse,
                        AngularUnits::Auto,
                    )
                    .expect("soa inverse");
            });
        });
    }
    group.finish();
}

fn bench_aos(c: &mut Criterion) {
    let mut group = c.benchmark_group("aos_xyz_roundtrip");
    for size in [64usize, 1024, 16_384] {
        group.throughput(Throughput::Elements(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, &size| {
            let context = Context::configured().expect("context");
            let mut transformer = transformer(&context);
            let mut coordinates = vec![[6378137.0, 1000.0, 20.0]; size];
            b.iter(|| {
                transformer
                    .transform_xyz_aos_in_place(
                        black_box(&mut coordinates),
                        Direction::Forward,
                        AngularUnits::Auto,
                    )
                    .expect("aos forward");
                transformer
                    .transform_xyz_aos_in_place(
                        black_box(&mut coordinates),
                        Direction::Inverse,
                        AngularUnits::Auto,
                    )
                    .expect("aos inverse");
            });
        });
    }
    group.finish();
}

fn bench_soa_forward(c: &mut Criterion) {
    let mut group = c.benchmark_group("soa_xyz_forward");
    for size in [64usize, 1024, 16_384, 65_536] {
        group.throughput(Throughput::Elements(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, &size| {
            let context = Context::configured().expect("context");
            let mut transformer = transformer(&context);
            b.iter_batched(
                || (vec![6378137.0; size], vec![1000.0; size], vec![20.0; size]),
                |(mut x, mut y, mut z)| {
                    transformer
                        .transform_xyz_in_place(
                            black_box(&mut x),
                            black_box(&mut y),
                            black_box(&mut z),
                            Direction::Forward,
                            AngularUnits::Auto,
                        )
                        .expect("soa forward");
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

fn bench_aos_forward(c: &mut Criterion) {
    let mut group = c.benchmark_group("aos_xyz_forward");
    for size in [64usize, 1024, 16_384, 65_536] {
        group.throughput(Throughput::Elements(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, &size| {
            let context = Context::configured().expect("context");
            let mut transformer = transformer(&context);
            b.iter_batched(
                || vec![[6378137.0, 1000.0, 20.0]; size],
                |mut coordinates| {
                    transformer
                        .transform_xyz_aos_in_place(
                            black_box(&mut coordinates),
                            Direction::Forward,
                            AngularUnits::Auto,
                        )
                        .expect("aos forward");
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

fn bench_geod(c: &mut Criterion) {
    let context = Context::configured().expect("context");
    let geod = Geod::wgs84(&context).expect("create benchmark geod");
    let mut group = c.benchmark_group("geod_inverse");
    for size in [64usize, 1024, 16_384] {
        group.throughput(Throughput::Elements(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, &size| {
            let first_longitudes = vec![0.0; size];
            let first_latitudes = vec![40.0; size];
            let second_longitudes = vec![1.0; size];
            let second_latitudes = vec![41.0; size];
            let mut distances = vec![0.0; size];
            let mut forward_azimuths = vec![0.0; size];
            let mut reverse_azimuths = vec![0.0; size];
            b.iter(|| {
                geod.inverse_batch_into(
                    black_box(&first_longitudes),
                    black_box(&first_latitudes),
                    black_box(&second_longitudes),
                    black_box(&second_latitudes),
                    black_box(&mut distances),
                    black_box(&mut forward_azimuths),
                    black_box(&mut reverse_azimuths),
                )
                .expect("geod batch");
            });
        });
    }
    group.finish();
}

fn bench_vertical_grid(c: &mut Criterion) {
    let Ok(grid_dir) = std::env::var("PROXI_BENCH_GRID_DIR") else {
        eprintln!("Skipping vertical_grid_xyz: set PROXI_BENCH_GRID_DIR to a local grid directory");
        return;
    };
    let context = Context::configured().expect("context");
    let mut transformer = TransformerBuilder::new(&context, "EPSG:4979", "EPSG:4326+5773")
        .context_options(ContextOptions::default().push_data_path(grid_dir))
        .allow_ballpark(false)
        .build()
        .expect("build vertical grid transformer");
    let grids = transformer.grids().expect("inspect vertical grids");
    assert!(
        !grids.is_empty() && grids.iter().all(|grid| grid.is_available()),
        "vertical benchmark requires all grids to be available: {grids:?}"
    );
    let size = 16_384usize;
    let mut x = vec![0.0; size];
    let mut y = vec![40.0; size];
    let mut z = vec![100.0; size];
    let mut group = c.benchmark_group("vertical_grid_xyz");
    group.throughput(Throughput::Elements(size as u64));
    group.bench_function("forward", |b| {
        b.iter(|| {
            transformer
                .transform_xyz_in_place(
                    black_box(&mut x),
                    black_box(&mut y),
                    black_box(&mut z),
                    Direction::Forward,
                    AngularUnits::Auto,
                )
                .expect("vertical grid transform");
        });
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_transformer_construction,
    bench_scalar_reuse,
    bench_soa,
    bench_aos,
    bench_soa_forward,
    bench_aos_forward,
    bench_geod,
    bench_vertical_grid
);
criterion_main!(benches);
