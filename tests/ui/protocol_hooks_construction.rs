//! Verifies the supported ways to construct and consume protocol hooks.
//!
//! The fixture pins four construction paths: independently registered
//! callbacks, a concrete protocol with a non-unit error type, an erased
//! protocol object, and the owned handle the application accessor returns.

use std::{io, sync::Arc};

use wireframe::{
    app::{Envelope, WireframeApp},
    codec::EofError,
    hooks::{ConnectionContext, CustomProtocolHooks, ProtocolHooks, WireframeProtocol},
    push::PushHandle,
    serializer::BincodeSerializer,
};

/// Protocol with a non-unit error type.
struct NonUnitErrorProtocol;

impl WireframeProtocol for NonUnitErrorProtocol {
    type Frame = Vec<u8>;
    type ProtocolError = io::Error;

    fn handle_error(&self, error: Self::ProtocolError, _ctx: &mut ConnectionContext) {
        drop(error);
    }
}

/// Accept the exact owned accessor shape an application exposes.
fn accept_owned<P>(protocol: Option<Arc<P>>) -> Option<Arc<P>>
where
    P: WireframeProtocol<ProtocolError = ()> + ?Sized,
{
    protocol
}

fn main() {
    // Independently registered callbacks convert into the explicit enum.
    let custom: ProtocolHooks<Vec<u8>, io::Error> = CustomProtocolHooks {
        on_connection_setup: Some(Box::new(
            |_handle: PushHandle<Vec<u8>>, _ctx: &mut ConnectionContext| {},
        )),
        before_send: Some(Box::new(|frame: &mut Vec<u8>, _ctx: &mut ConnectionContext| {
            frame.push(0);
        })),
        on_command_end: Some(Box::new(|_ctx: &mut ConnectionContext| {})),
        handle_error: Some(Box::new(|_error: io::Error, _ctx: &mut ConnectionContext| {})),
        stream_end: Some(Box::new(|_ctx: &mut ConnectionContext| Some(Vec::new()))),
        on_eof: Some(Box::new(
            |_error: &EofError, _partial: &[u8], _ctx: &mut ConnectionContext| {},
        )),
    }
    .into();
    let _ = custom;

    // A concrete protocol with a non-unit error type.
    let concrete = Arc::new(NonUnitErrorProtocol);
    let _: ProtocolHooks<Vec<u8>, io::Error> = ProtocolHooks::from_protocol(&concrete);

    // The same protocol entered through the erased trait object.
    let erased: Arc<dyn WireframeProtocol<Frame = Vec<u8>, ProtocolError = io::Error>> =
        Arc::clone(&concrete) as Arc<dyn WireframeProtocol<Frame = Vec<u8>, ProtocolError = io::Error>>;
    let _: ProtocolHooks<Vec<u8>, io::Error> = ProtocolHooks::from_protocol(&erased);

    // The application accessor returns an owned, erased protocol handle.
    let app: WireframeApp<BincodeSerializer, (), Envelope> =
        WireframeApp::new().expect("builder");
    let owned = accept_owned(app.protocol());
    assert!(owned.is_none(), "no protocol was installed");
}