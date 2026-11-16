//! Handler and middleware type aliases for `WireframeApp`.

use std::{future::Future, pin::Pin, sync::Arc};

use bytes::Bytes;

use super::envelope::Packet;
use crate::{
    middleware::{HandlerService, Transform},
    request::{RequestBodyStream, RequestParts},
};

/// Alias for asynchronous route handlers.
///
/// A `Handler` wraps an `Arc` to a function returning a [`Future`].
pub type Handler<E> = Arc<dyn Fn(&E) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// Alias for handlers that consume a streaming request body.
///
/// The handler receives routing metadata separately from the body and returns
/// an optional response payload. The payload is [`Bytes`] rather than
/// `Vec<u8>`: ADR 008 makes the zero-copy container the stable public payload
/// type, and a new public surface must not add another `Vec<u8>` hand-off that
/// the migration would later have to remove.
/// Returning `None` suppresses the response frame. Streaming handlers do not
/// run the middleware chain: middleware sees a whole payload, while a
/// streaming handler owns the body incrementally.
pub type StreamingHandler = Arc<
    dyn Fn(RequestParts, RequestBodyStream) -> Pin<Box<dyn Future<Output = Option<Bytes>> + Send>>
        + Send
        + Sync,
>;

/// Trait representing middleware components.
pub trait Middleware<E: Packet>:
    Transform<HandlerService<E>, Output = HandlerService<E>> + Send + Sync
{
}

impl<E: Packet, T> Middleware<E> for T where
    T: Transform<HandlerService<E>, Output = HandlerService<E>> + Send + Sync
{
}
