//! Preamble configuration methods for `WireframeClientBuilder`.

use bincode::Encode;

use super::WireframeClientBuilder;
use crate::{client::preamble_exchange::PreambleConfig, serializer::Serializer};

impl<S, P, C> WireframeClientBuilder<S, P, C>
where
    S: Serializer + Send + Sync,
{
    /// Configure a preamble to send before exchanging frames.
    ///
    /// The preamble is written to the server immediately after establishing
    /// the TCP connection, before the framing layer begins. Use
    /// [`on_preamble_success`](Self::on_preamble_success) to read the server's
    /// response and [`preamble_timeout`](Self::preamble_timeout) to bound the
    /// exchange.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::client::WireframeClientBuilder;
    ///
    /// #[derive(bincode::Encode)]
    /// struct MyPreamble {
    ///     version: u16,
    /// }
    ///
    /// let builder = WireframeClientBuilder::new().with_preamble(MyPreamble { version: 1 });
    /// let _ = builder;
    /// ```
    #[must_use]
    pub fn with_preamble<Q>(self, preamble: Q) -> WireframeClientBuilder<S, Q, C>
    where
        Q: Encode + Send + Sync + 'static,
    {
        builder_field_update!(self, preamble_config = Some(PreambleConfig::new(preamble)))
    }
}
