//! Preamble configuration for [`WireframeServer`].

use core::marker::PhantomData;

use bincode::error::DecodeError;

use super::WireframeServer;
use crate::{
    app::WireframeApp,
    preamble::Preamble,
    server::{PreambleSuccessHandler, ServerState},
};

impl<F, T, S> WireframeServer<F, T, S>
where
    F: Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    T: Preamble,
    S: ServerState,
{
    /// Converts the server to use a custom preamble type implementing
    /// [`crate::preamble::Preamble`] for incoming connections.
    ///
    /// Calling this method drops any previously configured preamble decode callbacks
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

    builder_callback!(
        /// Register a callback invoked when the connection preamble decodes successfully.
        ///
        /// The handler must implement [`crate::server::PreambleSuccessHandler`].
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

    builder_callback!(
        /// Register a callback invoked when the connection preamble fails to decode.
        ///
        /// The handler receives a [`bincode::error::DecodeError`].
        ///
        /// # Examples
        ///
        /// ```
        /// use wireframe::{app::WireframeApp, server::WireframeServer};
        ///
        /// let server = WireframeServer::new(|| WireframeApp::default()).on_preamble_decode_failure(
        ///     |_err: &bincode::error::DecodeError| {
        ///         eprintln!("Failed to decode preamble");
        ///     },
        /// );
        /// ```
        on_preamble_decode_failure,
        on_preamble_failure,
        Fn(&DecodeError) + Send + Sync + 'static
    );
}
