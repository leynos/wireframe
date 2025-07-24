use cucumber::{given, then, when};
use tokio::{net::TcpStream, sync::oneshot};
use wireframe::{app::WireframeApp, server::WireframeServer};

use crate::world::PanicWorld;

#[given("a running wireframe server with a panic in connection setup")]
async fn start_server(world: &mut PanicWorld) {
    let factory = || {
        WireframeApp::new()
            .unwrap()
            .on_connection_setup(|| async { panic!("boom") })
            .unwrap()
    };
    let server = WireframeServer::new(factory)
        .workers(1)
        .bind("127.0.0.1:0".parse().unwrap())
        .expect("bind");
    world.addr = Some(server.local_addr().unwrap());
    let (tx, rx) = oneshot::channel();
    world.shutdown = Some(tx);
    tokio::spawn(async move {
        server
            .run_with_shutdown(async {
                let _ = rx.await;
            })
            .await
            .unwrap();
    });
    tokio::task::yield_now().await;
}

#[when("I connect to the server")]
async fn connect(world: &mut PanicWorld) {
    TcpStream::connect(world.addr.unwrap()).await.unwrap();
    world.attempts += 1;
}

#[then("both connections succeed")]
async fn verify(world: &mut PanicWorld) {
    assert_eq!(world.attempts, 2);
    if let Some(tx) = world.shutdown.take() {
        let _ = tx.send(());
    }
    tokio::task::yield_now().await;
}
