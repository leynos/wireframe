//! Preamble configuration for [`WireframeServer`].

use core::marker::PhantomData;
use std::{io, sync::Arc};

use bincode::error::DecodeError;
use futures::future::BoxFuture;

use super::WireframeServer;
use crate::{app::WireframeApp, preamble::Preamble, server::ServerState};

impl<F, T, S> WireframeServer<F, T, S>
where
    F: Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    T: Preamble,
    S: ServerState,
{
    /// Converts the server to use a custom preamble type for incoming connections.
    ///
    /// Calling this method drops any previously configured preamble decode callbacks.
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
    pub fn with_preamble<P>(self) -> WireframeServer<F, P, S>
    where
        P: Preamble,
    {
        WireframeServer {
            factory: self.factory,
            workers: self.workers,
            on_preamble_success: None,
            on_preamble_failure: None,
            ready_tx: self.ready_tx,
            state: self.state,
            _preamble: PhantomData,
        }
    }

    /// Register a callback invoked when the connection preamble decodes successfully.
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
    #[must_use]
    pub fn on_preamble_decode_success<H>(mut self, handler: H) -> Self
    where
        H: for<'a> Fn(&'a T, &'a mut tokio::net::TcpStream) -> BoxFuture<'a, io::Result<()>>
            + Send
            + Sync
            + 'static,
    {
        self.on_preamble_success = Some(Arc::new(handler));
        self
    }

    /// Register a callback invoked when the connection preamble fails to decode.
    ///
    /// # Examples
    ///
    /// ```
    /// use bincode::error::DecodeError;
    /// use wireframe::{app::WireframeApp, server::WireframeServer};
    ///
    /// let server = WireframeServer::new(|| WireframeApp::default()).on_preamble_decode_failure(
    ///     |_err: &DecodeError| {
    ///         eprintln!("Failed to decode preamble");
    ///     },
    /// );
    /// ```
    #[must_use]
    pub fn on_preamble_decode_failure<H>(mut self, handler: H) -> Self
    where
        H: Fn(&DecodeError) + Send + Sync + 'static,
    {
        self.on_preamble_failure = Some(Arc::new(handler));
        self
    }
}
