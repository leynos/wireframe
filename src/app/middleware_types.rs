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
pub type ResponseHandler<E> =
    Arc<dyn Fn(&E) -> Pin<Box<dyn Future<Output = Response<Envelope, ()>> + Send>> + Send + Sync>;

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
