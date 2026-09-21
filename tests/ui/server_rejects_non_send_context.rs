//! Reject a server application whose connection state cannot cross task boundaries.

use std::rc::Rc;

use wireframe::{
    app::{Envelope, WireframeApp},
    serializer::BincodeSerializer,
    server::WireframeServer,
};

fn main() {
    let app = WireframeApp::<BincodeSerializer, Rc<()>, Envelope>::new().expect("application");
    let _server = WireframeServer::from_app(app);
}
