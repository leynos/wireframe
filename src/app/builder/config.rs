//! General configuration methods for `WireframeApp`.

use tokio::sync::mpsc;

use super::WireframeApp;
use crate::{
    app::{
        Packet,
        builder_defaults::{MAX_READ_TIMEOUT_MS, MIN_READ_TIMEOUT_MS, default_fragmentation},
    },
    codec::FrameCodec,
    fragment::FragmentationConfig,
    serializer::Serializer,
};

impl<S, C, E, F> WireframeApp<S, C, E, F>
where
    S: Serializer + Send + Sync,
    C: Send + 'static,
    E: Packet,
    F: FrameCodec,
{
    /// Configure the read timeout in milliseconds.
    /// Clamped between 1 and 86,400,000 milliseconds (24 h).
    #[must_use]
    pub fn read_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.read_timeout_ms = timeout_ms.clamp(MIN_READ_TIMEOUT_MS, MAX_READ_TIMEOUT_MS);
        self
    }

    /// Override the fragmentation configuration.
    ///
    /// Provide `None` to disable fragmentation entirely.
    #[must_use]
    pub fn fragmentation(mut self, config: Option<FragmentationConfig>) -> Self {
        self.fragmentation = config;
        self
    }

    /// Enable transport fragmentation using codec-derived defaults.
    ///
    /// The derived settings are bounded by the current frame codec budget.
    /// Call this after `with_codec` or `buffer_capacity` so defaults align with
    /// the final frame length.
    #[must_use]
    pub fn enable_fragmentation(mut self) -> Self {
        self.fragmentation = default_fragmentation(self.codec.max_frame_length());
        self
    }

    /// Configure a Dead Letter Queue for dropped push frames.
    ///
    /// ```rust,no_run
    /// use tokio::sync::mpsc;
    /// use wireframe::app::WireframeApp;
    ///
    /// # fn build() -> WireframeApp {
    /// #     WireframeApp::new().expect("builder creation should not fail")
    /// # }
    /// # fn main() {
    /// let (tx, _rx) = mpsc::channel(16);
    /// let app = build().with_push_dlq(tx);
    /// # let _ = app;
    /// # }
    /// ```
    #[must_use]
    pub fn with_push_dlq(self, dlq: mpsc::Sender<Vec<u8>>) -> Self {
        WireframeApp {
            push_dlq: Some(dlq),
            ..self
        }
    }
}
