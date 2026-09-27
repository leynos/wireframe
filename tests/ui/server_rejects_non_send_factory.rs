//! Reject a server factory that cannot cross connection task boundaries.

use std::rc::Rc;

use wireframe::{app::WireframeApp, server::WireframeServer};

fn main() {
    let captured = Rc::new(());
    let _server = WireframeServer::new(move || {
        let _ = &captured;
        WireframeApp::default()
    });
}
