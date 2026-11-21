//! Coverage for the identifier space shared by buffered and streaming routes.
//!
//! A buffered route and a streaming route answer the same inbound message
//! identifier, so they cannot be registered independently: whichever is
//! registered first owns the identifier, and the second registration must be
//! rejected rather than silently shadowing it.

use std::sync::Arc;

use rstest::rstest;

use crate::{
    WireframeError,
    app::{Envelope, Handler, StreamingHandler, WireframeApp},
    request::{RequestBodyStream, RequestParts},
};

/// Identifier used by every duplicate-registration case.
const ROUTE_ID: u32 = 7;

/// Which route map a registration targets.
#[derive(Clone, Copy, Debug)]
enum RouteKind {
    /// A buffered route whose handler ignores its request.
    Buffered,
    /// A streaming route whose handler reads nothing and replies nothing.
    Streaming,
}

/// Register `kind` against [`ROUTE_ID`] on `app`.
fn register(app: WireframeApp, kind: RouteKind) -> Result<WireframeApp, WireframeError> {
    match kind {
        RouteKind::Buffered => app.route(ROUTE_ID, buffered_handler()),
        RouteKind::Streaming => app.streaming_route(ROUTE_ID, streaming_handler()),
    }
}

/// A handler that accepts an envelope without changing it.
fn buffered_handler() -> Handler<Envelope> { Arc::new(|_: &Envelope| Box::pin(async {})) }

/// A handler that accepts a body stream and never replies.
fn streaming_handler() -> StreamingHandler {
    Arc::new(|_parts: RequestParts, _body: RequestBodyStream| Box::pin(async { None }))
}

/// Registering one identifier in both kinds of route map is rejected.
#[rstest]
#[case::buffered_then_streaming(RouteKind::Buffered, RouteKind::Streaming)]
#[case::streaming_then_buffered(RouteKind::Streaming, RouteKind::Buffered)]
#[case::streaming_twice(RouteKind::Streaming, RouteKind::Streaming)]
#[case::buffered_twice(RouteKind::Buffered, RouteKind::Buffered)]
fn registering_one_identifier_twice_is_rejected(
    #[case] first: RouteKind,
    #[case] second: RouteKind,
) {
    let app = WireframeApp::new().expect("the default application should build");
    let registered = register(app, first).expect("the first registration should succeed");

    let Err(error) = register(registered, second) else {
        panic!("the second registration must be rejected");
    };

    assert!(
        matches!(error, WireframeError::DuplicateRoute(id) if id == ROUTE_ID),
        "a duplicate identifier must be reported as DuplicateRoute, got {error:?}"
    );
}
