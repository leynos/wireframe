//! Preamble configuration extension for the wireframe client builder.
//!
//! This module provides the [`ClientPreambleExt`] trait that adds preamble
//! configuration methods to [`WireframeClientBuilder`].

use std::{io, time::Duration};

use bincode::Encode;
use futures::future::BoxFuture;

use super::{ClientError, WireframeClientBuilder};
use crate::serializer::Serializer;

impl<S, P, C> WireframeClientBuilder<S, P, C>
where
    S: Serializer + Send + Sync,
    P: Encode + Send + Sync + 'static,
{
    /// Configure a timeout for the preamble exchange.
    ///
    /// The timeout is applied to the entire preamble phase: writing the
    /// preamble and running the success callback (which may read the server's
    /// response). Values below 1 ms are clamped to 1 ms to avoid immediate
    /// expiry.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    ///
    /// use wireframe::client::WireframeClientBuilder;
    ///
    /// #[derive(bincode::Encode)]
    /// struct MyPreamble(u8);
    ///
    /// let builder = WireframeClientBuilder::new()
    ///     .with_preamble(MyPreamble(1))
    ///     .preamble_timeout(Duration::from_secs(1));
    /// let _ = builder;
    /// ```
    #[must_use]
    pub fn preamble_timeout(mut self, duration: Duration) -> Self {
        let normalised = duration.max(Duration::from_millis(1));
        if let Some(ref mut config) = self.preamble_config {
            config.set_timeout(normalised);
        }
        self
    }

    /// Register a handler invoked after the preamble is successfully written.
    ///
    /// The handler receives the sent preamble and a mutable reference to the
    /// TCP stream. It may read the server's response preamble from the stream.
    /// Any leftover bytes (read beyond the server's response) must be returned
    /// so they can be replayed before framed communication begins.
    ///
    /// # Examples
    ///
    /// ```
    /// use futures::FutureExt;
    /// use wireframe::client::WireframeClientBuilder;
    ///
    /// #[derive(bincode::Encode)]
    /// struct MyPreamble(u8);
    ///
    /// let builder = WireframeClientBuilder::new()
    ///     .with_preamble(MyPreamble(1))
    ///     .on_preamble_success(|_preamble, _stream| {
    ///         async move {
    ///             // Read server response if needed...
    ///             Ok(Vec::new()) // No leftover bytes
    ///         }
    ///         .boxed()
    ///     });
    /// let _ = builder;
    /// ```
    #[must_use]
    pub fn on_preamble_success<H>(mut self, handler: H) -> Self
    where
        H: for<'a> Fn(&'a P, &'a mut tokio::net::TcpStream) -> BoxFuture<'a, io::Result<Vec<u8>>>
            + Send
            + Sync
            + 'static,
    {
        if let Some(ref mut config) = self.preamble_config {
            config.set_success_handler(handler);
        }
        self
    }

    /// Register a handler invoked when the preamble exchange fails.
    ///
    /// The handler receives the error and a mutable reference to the TCP
    /// stream, allowing it to log or send an error response before the
    /// connection closes.
    ///
    /// # Examples
    ///
    /// ```
    /// use futures::FutureExt;
    /// use wireframe::client::WireframeClientBuilder;
    ///
    /// #[derive(bincode::Encode)]
    /// struct MyPreamble(u8);
    ///
    /// let builder = WireframeClientBuilder::new()
    ///     .with_preamble(MyPreamble(1))
    ///     .on_preamble_failure(|err, _stream| {
    ///         async move {
    ///             eprintln!("Preamble failed: {err}");
    ///             Ok(())
    ///         }
    ///         .boxed()
    ///     });
    /// let _ = builder;
    /// ```
    #[must_use]
    pub fn on_preamble_failure<H>(mut self, handler: H) -> Self
    where
        H: for<'a> Fn(
                &'a ClientError,
                &'a mut tokio::net::TcpStream,
            ) -> BoxFuture<'a, io::Result<()>>
            + Send
            + Sync
            + 'static,
    {
        if let Some(ref mut config) = self.preamble_config {
            config.set_failure_handler(handler);
        }
        self
    }
}
