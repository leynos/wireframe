//! Criterion coverage for application factory and preparation startup work.

use std::{
    hint::black_box,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use async_trait::async_trait;
use criterion::{Criterion, criterion_group, criterion_main};
use tokio::runtime::Runtime;
use wireframe::{
    app::{Envelope, Handler, Result as AppResult, WireframeApp},
    middleware::{HandlerService, Transform},
};

/// Fixed connection count used to contrast per-connection and shared startup.
const CONNECTIONS: usize = 4;

/// Counters kept outside the timed result so both startup topologies are
/// observable in Criterion's benchmark identifiers.
#[derive(Clone)]
struct StartupCounters {
    /// Number of factory evaluations.
    factory: Arc<AtomicUsize>,
    /// Number of middleware transformations.
    transforms: Arc<AtomicUsize>,
}

impl StartupCounters {
    /// Create zeroed counters for one benchmark sample.
    fn new() -> Self {
        Self {
            factory: Arc::new(AtomicUsize::new(0)),
            transforms: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Return the completed sample's factory and transform totals.
    fn snapshot(&self) -> (usize, usize) {
        (
            self.factory.load(Ordering::SeqCst),
            self.transforms.load(Ordering::SeqCst),
        )
    }
}

/// Middleware whose only effect is recording preparation work.
struct CountingTransform {
    /// Counter shared with the benchmark sample.
    transforms: Arc<AtomicUsize>,
}

#[async_trait]
impl Transform<HandlerService<Envelope>> for CountingTransform {
    type Output = HandlerService<Envelope>;

    /// Count preparation before returning the unmodified route service.
    async fn transform(&self, service: HandlerService<Envelope>) -> Self::Output {
        self.transforms.fetch_add(1, Ordering::SeqCst);
        service
    }
}

/// Build the single route required to make middleware preparation observable.
fn build_app(counters: &StartupCounters) -> AppResult<WireframeApp> {
    let handler: Handler<Envelope> = Arc::new(|_: &Envelope| Box::pin(async {}));
    counters.factory.fetch_add(1, Ordering::SeqCst);
    WireframeApp::new()?
        .route(1, handler)?
        .wrap(CountingTransform {
            transforms: Arc::clone(&counters.transforms),
        })
}

/// Build the benchmark app or stop when its static setup becomes invalid.
fn build_app_or_panic(counters: &StartupCounters) -> WireframeApp {
    expect_bench(
        build_app(counters),
        "connection-startup benchmark app failed",
    )
}

/// Prepare a freshly built app or stop the benchmark if its setup is invalid.
fn prepare(runtime: &Runtime, app: WireframeApp) {
    black_box(expect_bench(
        runtime.block_on(app.prepare()),
        "connection-startup benchmark setup failed",
    ));
}

/// Extract a benchmark setup result at Criterion's panic boundary.
///
/// Criterion iteration closures cannot return `Result`; invalid setup cannot
/// produce a meaningful measurement, so this helper terminates the benchmark
/// with a clear diagnostic.
fn expect_bench<T, E: std::fmt::Display>(result: Result<T, E>, context: &str) -> T {
    match result {
        Ok(value) => value,
        Err(error) => panic!("{context}: {error}"),
    }
}

/// Compare the old per-connection path with one shared prepared root.
fn benchmark_connection_startup(criterion: &mut Criterion) {
    let runtime = expect_bench(
        Runtime::new(),
        "connection-startup benchmark runtime failed",
    );
    let mut group = criterion.benchmark_group("server/connection_startup");

    group.bench_function("per_connection_factory_4_transforms_4", |bencher| {
        bencher.iter(|| {
            let counters = StartupCounters::new();
            for _ in 0..CONNECTIONS {
                let app = build_app_or_panic(&counters);
                prepare(&runtime, app);
            }
            black_box(counters.snapshot());
        });
    });

    group.bench_function("prepared_root_factory_1_transforms_1", |bencher| {
        bencher.iter(|| {
            let counters = StartupCounters::new();
            let app = build_app_or_panic(&counters);
            prepare(&runtime, app);
            for _ in 0..CONNECTIONS {
                black_box(counters.snapshot());
            }
            black_box(counters.snapshot());
        });
    });

    group.finish();
}

criterion_group!(benches, benchmark_connection_startup);
criterion_main!(benches);
