//! Handler and middleware type aliases for `WireframeApp`.

use std::{future::Future, pin::Pin, sync::Arc};

use super::envelope::Packet;
use crate::middleware::{HandlerService, Transform};

/// Alias for asynchronous route handlers.
///
/// A `Handler` wraps an `Arc` to a function returning a [`Future`].
pub type Handler<E> = Arc<dyn Fn(&E) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// Trait representing middleware components.
pub trait Middleware<E: Packet>:
    Transform<HandlerService<E>, Output = HandlerService<E>> + Send + Sync
{
}

impl<E: Packet, T> Middleware<E> for T where
    T: Transform<HandlerService<E>, Output = HandlerService<E>> + Send + Sync
{
}
