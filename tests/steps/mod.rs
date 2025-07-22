use cucumber::{given, then, when};
use tokio_util::sync::CancellationToken;
use wireframe::{connection::ConnectionActor, push::PushQueues};

use crate::MetricsWorld;

#[given("a metrics recorder")]
fn metrics_recorder(world: &mut MetricsWorld) {
    let handle = metrics_exporter_prometheus::PrometheusBuilder::new()
        .install_recorder()
        .expect("recorder install");
    world.exporter = Some(handle);
}

#[when("a connection runs with no frames")]
async fn run_connection(world: &mut MetricsWorld) {
    let (queues, handle) = PushQueues::<u8>::bounded(1, 1);
    let token = CancellationToken::new();
    let mut actor: ConnectionActor<_, ()> = ConnectionActor::new(queues, handle, None, token);
    let mut out = Vec::new();
    actor.run(&mut out).await.unwrap();
    if let Some(h) = &world.exporter {
        h.run_upkeep();
        world.output = Some(h.render());
    }
}

#[then("the exporter output includes active connections gauge")]
fn check_output(world: &mut MetricsWorld) {
    let out = world.output.as_ref().expect("no output");
    assert!(out.contains("wireframe_connections_active"), "{out}");
}
