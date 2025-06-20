use std::io;

use wireframe::{
    app::{Middleware, WireframeApp},
    server::WireframeServer,
};

struct Logger;
impl Middleware for Logger {}

#[tokio::main]
async fn main() -> io::Result<()> {
    let factory = || {
        WireframeApp::new()
            .unwrap()
            .wrap(Logger)
            .unwrap()
            .route(1, Box::new(|_env| Box::pin(async {})))
            .unwrap()
    };

    WireframeServer::new(factory)
        .bind("127.0.0.1:7878".parse().unwrap())?
        .run()
        .await
}
