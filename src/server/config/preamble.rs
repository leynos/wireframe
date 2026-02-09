//! Preamble configuration for [`WireframeServer`].

use core::marker::PhantomData;
use std::{io, time::Duration};

use bincode::error::DecodeError;
use futures::future::BoxFuture;

use super::WireframeServer;
use crate::{
    app::Packet,
    codec::FrameCodec,
    preamble::Preamble,
    serializer::Serializer,
    server::{AppFactory, PreambleSuccessHandler, ServerState},
};

impl<F, T, S, Ser, Ctx, E, Codec> WireframeServer<F, T, S, Ser, Ctx, E, Codec>
where
    F: AppFactory<Ser, Ctx, E, Codec>,
    T: Preamble,
    S: ServerState,
    Ser: Serializer + Send + Sync,
    Ctx: Send + 'static,
    E: Packet,
    Codec: FrameCodec,
{
    /// Converts the server to use a custom preamble type implementing
    /// [`crate::preamble::Preamble`] for incoming connections.
    ///
    /// Calling this method drops any previously configured preamble handlers
    /// (both success and failure).
    ///
    /// # Examples
    ///
    /// ```
    /// use bincode::{Decode, Encode};
    /// use wireframe::{app::WireframeApp, preamble::Preamble, server::WireframeServer};
    ///
    /// #[derive(Encode, Decode)]
    /// struct MyPreamble;
    /// impl Preamble for MyPreamble {}
    ///
    /// let server = WireframeServer::new(|| WireframeApp::default()).with_preamble::<MyPreamble>();
    /// ```
    #[must_use]
    pub fn with_preamble<P>(self) -> WireframeServer<F, P, S, Ser, Ctx, E, Codec>
    where
        P: Preamble,
    {
        WireframeServer {
            factory: self.factory,
            workers: self.workers,
            on_preamble_success: None,
            on_preamble_failure: None,
            ready_tx: self.ready_tx,
            backoff_config: self.backoff_config,
            preamble_timeout: self.preamble_timeout,
            state: self.state,
            _app: self._app,
            _preamble: PhantomData,
        }
    }

    /// Configure a timeout for reading the connection preamble.
    ///
    /// The timeout is applied around the preamble read. When it elapses, the
    /// preamble decode failure handler is invoked (if registered) and the
    /// connection is closed. Values below 1 ms are clamped to 1 ms to avoid
    /// immediate expiry. Omit this setter to disable the timeout.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    ///
    /// use wireframe::{app::WireframeApp, server::WireframeServer};
    ///
    /// let server =
    ///     WireframeServer::new(|| WireframeApp::default()).preamble_timeout(Duration::from_secs(1));
    /// ```
    #[must_use]
    pub fn preamble_timeout(mut self, timeout: Duration) -> Self {
        let normalised = timeout.max(Duration::from_millis(1));
        self.preamble_timeout = Some(normalised);
        self
    }

    builder_callback!(
        /// Register a handler invoked when the connection preamble decodes successfully.
        ///
        /// The handler must implement [`crate::server::PreambleSuccessHandler`].
        /// See [`crate::server::PreambleHandler`] for a ready-to-use alias.
        ///
        /// # Examples
        ///
        /// ```
        /// use bincode::{Decode, Encode};
        /// use futures::FutureExt;
        /// use wireframe::{app::WireframeApp, preamble::Preamble, server::WireframeServer};
        ///
        /// #[derive(Encode, Decode)]
        /// struct MyPreamble;
        /// impl Preamble for MyPreamble {}
        ///
        /// let server = WireframeServer::new(|| WireframeApp::default())
        ///     .with_preamble::<MyPreamble>()
        ///     .on_preamble_decode_success(|_p: &MyPreamble, _s| async { Ok(()) }.boxed());
        /// ```
        on_preamble_decode_success,
        on_preamble_success,
        PreambleSuccessHandler<T>
    );

    /// Register a handler invoked when the connection preamble fails to decode.
    ///
    /// The handler receives a [`bincode::error::DecodeError`] and a mutable
    /// [`tokio::net::TcpStream`] so it may emit a response before the
    /// connection is closed. This callback is awaited; if it returns an
    /// error, the error is logged and the connection is closed.
    ///
    /// # Examples
    ///
    /// ```
    /// use futures::FutureExt;
    /// use tokio::io::AsyncWriteExt;
    /// use wireframe::{app::WireframeApp, server::WireframeServer};
    ///
    /// let server = WireframeServer::new(|| WireframeApp::default()).on_preamble_decode_failure(
    ///     |_err: &bincode::error::DecodeError, stream| {
    ///         async move {
    ///             stream.write_all(b"BAD").await?;
    ///             Ok(())
    ///         }
    ///         .boxed()
    ///     },
    /// );
    /// ```
    #[must_use]
    pub fn on_preamble_decode_failure<H>(mut self, handler: H) -> Self
    where
        H: for<'a> Fn(
                &'a DecodeError,
                &'a mut tokio::net::TcpStream,
            ) -> BoxFuture<'a, io::Result<()>>
            + Send
            + Sync
            + 'static,
    {
        self.on_preamble_failure = Some(std::sync::Arc::new(handler));
        self
    }
}
