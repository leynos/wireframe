//! Verifies a pre-built application can construct an unbound server.

use wireframe::{app::WireframeApp, server::WireframeServer};

fn main() {
    let _: WireframeServer<_> = WireframeServer::from_app(WireframeApp::default());
}
