//! Tracing configuration builder method for [`WireframeClientBuilder`].

use super::WireframeClientBuilder;
use crate::{client::tracing_config::TracingConfig, serializer::Serializer};

impl<S, P, C> WireframeClientBuilder<S, P, C>
where
    S: Serializer + Send + Sync,
{
    /// Configure tracing instrumentation for the client.
    ///
    /// The [`TracingConfig`] controls which operations emit tracing spans,
    /// at what level, and whether per-command elapsed-time events are
    /// recorded.
    ///
    /// When not called, the client uses [`TracingConfig::default()`], which
    /// emits `INFO` spans for lifecycle operations (`connect`, `close`) and
    /// `DEBUG` spans for data operations (`send`, `receive`, `call`,
    /// `call_streaming`). Timing is disabled by default.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::client::{TracingConfig, WireframeClientBuilder};
    ///
    /// let config = TracingConfig::default().with_all_timing(true);
    /// let builder = WireframeClientBuilder::new().tracing_config(config);
    /// let _ = builder;
    /// ```
    #[must_use]
    pub fn tracing_config(mut self, config: TracingConfig) -> Self {
        self.tracing_config = config;
        self
    }
}
