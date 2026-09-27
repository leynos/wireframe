//! Verifies supported startup factory result forms construct servers.

use std::convert::Infallible;

use wireframe::{app::WireframeApp, server::WireframeServer};

fn main() {
    let _: WireframeServer<_> = WireframeServer::new(WireframeApp::default);
    let _: WireframeServer<_> = WireframeServer::new(|| -> Result<WireframeApp, Infallible> {
        Ok(WireframeApp::default())
    });
}
