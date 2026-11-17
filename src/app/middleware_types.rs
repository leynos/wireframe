//! Handler and middleware type aliases for `WireframeApp`.

use std::{future::Future, pin::Pin, sync::Arc};

use super::envelope::{Envelope, Packet};
use crate::{
    middleware::{HandlerService, Transform},
    response::Response,
};

/// Alias for asynchronous route handlers.
///
/// A `Handler` wraps an `Arc` to a function returning a [`Future`]. It mutates
/// the request envelope in place, and the route service replies with the
/// envelope's final payload.
pub type Handler<E> = Arc<dyn Fn(&E) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// Alias for route handlers that build an explicit [`Response`].
///
/// Unlike [`Handler`], the returned future owns the frames the connection
/// streams to the client. Frames are [`Envelope`] values so they pass through
/// the connection's serializer, codec, and protocol hooks. Handlers may return
/// [`Response::MultiPacket`] to spawn a producer and stream frames
/// asynchronously, which is how a route keeps producing after it returns.
///
/// The returned future may borrow the request for as long as the handler call
/// lasts, so a handler can read request data after an `await` without copying
/// it first.
pub type ResponseHandler<E> = Arc<
    dyn for<'request> Fn(
            &'request E,
        )
            -> Pin<Box<dyn Future<Output = Response<Envelope, ()>> + Send + 'request>>
        + Send
        + Sync,
>;

/// Registered route kind stored by [`WireframeApp`].
///
/// [`WireframeApp`]: super::builder::WireframeApp
pub(crate) enum Route<E: Packet> {
    /// A unit handler mutates the request envelope in place.
    Unit(Handler<E>),
    /// A response handler returns the frames to stream to the client.
    Response(ResponseHandler<E>),
}

/// Trait representing middleware components.
pub trait Middleware<E: Packet>:
    Transform<HandlerService<E>, Output = HandlerService<E>> + Send + Sync
{
}

impl<E: Packet, T> Middleware<E> for T where
    T: Transform<HandlerService<E>, Output = HandlerService<E>> + Send + Sync
{
}

#[cfg(test)]
mod tests {
    //! Compile-time and runtime coverage for handler lifetime bounds.

    use super::*;

    /// Echo the request's payload back after yielding once.
    ///
    /// The function reads the request after an `await`, so installing it as a
    /// [`ResponseHandler`] only type-checks because the alias ties the returned
    /// future's lifetime to the borrowed request.
    async fn echo_after_yield(request: &Envelope) -> Response<Envelope, ()> {
        tokio::task::yield_now().await;
        Response::Single(Envelope::new(request.id, None, request.payload.clone()))
    }

    /// A response handler may read request data after an `await` without
    /// copying it first.
    #[tokio::test]
    async fn response_handler_may_borrow_the_request() {
        let handler: ResponseHandler<Envelope> =
            Arc::new(|request: &Envelope| Box::pin(echo_after_yield(request)));
        let request = Envelope::new(7, None, vec![1]);

        let response = handler(&request).await;

        assert!(
            matches!(response, Response::Single(frame) if frame.id == 7),
            "the handler must see the borrowed request after awaiting"
        );
    }
}
