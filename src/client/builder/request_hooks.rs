//! Request hook methods for `WireframeClientBuilder`.

use std::sync::Arc;

use super::WireframeClientBuilder;
use crate::serializer::Serializer;

impl<S, P, C> WireframeClientBuilder<S, P, C>
where
    S: Serializer + Send + Sync,
{
    /// Register a hook invoked after serialisation, before each frame is sent.
    ///
    /// Multiple hooks may be registered; they execute in registration order.
    /// Each hook receives a mutable reference to the serialised bytes, allowing
    /// inspection or modification (e.g., prepending an authentication token,
    /// incrementing a metrics counter).
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::{
    ///     Arc,
    ///     atomic::{AtomicUsize, Ordering},
    /// };
    ///
    /// use wireframe::client::WireframeClientBuilder;
    ///
    /// let counter = Arc::new(AtomicUsize::new(0));
    /// let count = counter.clone();
    /// let builder = WireframeClientBuilder::new().before_send(move |_bytes: &mut Vec<u8>| {
    ///     count.fetch_add(1, Ordering::Relaxed);
    /// });
    /// let _ = builder;
    /// ```
    #[must_use]
    pub fn before_send<F>(mut self, f: F) -> Self
    where
        F: Fn(&mut Vec<u8>) + Send + Sync + 'static,
    {
        self.request_hooks.before_send.push(Arc::new(f));
        self
    }

    /// Register a hook invoked after each frame is read, before
    /// deserialisation.
    ///
    /// Multiple hooks may be registered; they execute in registration order.
    /// Each hook receives a mutable reference to the raw frame bytes, allowing
    /// inspection or modification before the deserialiser processes them.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::{
    ///     Arc,
    ///     atomic::{AtomicUsize, Ordering},
    /// };
    ///
    /// use wireframe::client::WireframeClientBuilder;
    ///
    /// let counter = Arc::new(AtomicUsize::new(0));
    /// let count = counter.clone();
    /// let builder =
    ///     WireframeClientBuilder::new().after_receive(move |_bytes: &mut bytes::BytesMut| {
    ///         count.fetch_add(1, Ordering::Relaxed);
    ///     });
    /// let _ = builder;
    /// ```
    #[must_use]
    pub fn after_receive<F>(mut self, f: F) -> Self
    where
        F: Fn(&mut bytes::BytesMut) + Send + Sync + 'static,
    {
        self.request_hooks.after_receive.push(Arc::new(f));
        self
    }
}
