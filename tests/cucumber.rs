use cucumber::World as _;

#[derive(Debug, Default, cucumber::World)]
pub struct MetricsWorld {
    pub exporter: Option<metrics_exporter_prometheus::PrometheusHandle>,
    pub output: Option<String>,
}

mod steps;

#[tokio::main]
async fn main() { MetricsWorld::run("tests/features").await; }
