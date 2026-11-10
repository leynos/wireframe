//! Push-queue and session-registry configuration for `WireframeApp`.

use std::sync::Arc;

use tokio::sync::mpsc;

use super::WireframeApp;
use crate::{
    app::{Packet, PrepareError},
    codec::FrameCodec,
    push::{FrameLike, PushQueuesBuilder},
    serializer::Serializer,
    session::SessionRegistry,
};

/// Reject a stored push-queue template before any connection exists.
///
/// Validating during preparation turns a misconfigured capacity or rate into a
/// preparation error rather than a failure repeated by every accepted
/// connection.
///
/// # Errors
///
/// Returns [`PrepareError::InvalidPushConfig`] when the template is unusable.
pub(in crate::app) fn validate_push_queues<Fr>(
    push_queues: Option<&PushQueuesBuilder<Fr>>,
) -> Result<(), PrepareError>
where
    Fr: FrameLike,
{
    let Some(template) = push_queues else {
        return Ok(());
    };
    template
        .validate()
        .map_err(|source| PrepareError::InvalidPushConfig { source })
}

/// Warn once when an application still configures the legacy DLQ.
///
/// The legacy sink is not wired to connection queues, so silently ignoring it
/// would hide dropped frames from the operator.
pub(in crate::app) fn warn_about_legacy_push_dlq(push_dlq: Option<&mpsc::Sender<Vec<u8>>>) {
    if push_dlq.is_some() {
        tracing::warn!(
            "with_push_dlq configures an unused legacy sink; use with_push_queues to receive \
             dropped frames as codec frames"
        );
    }
}

impl<S, C, E, F> WireframeApp<S, C, E, F>
where
    S: Serializer + Send + Sync,
    C: Send + 'static,
    E: Packet,
    F: FrameCodec,
{
    /// Provide the template used to build every connection's push queues.
    ///
    /// The template is cloned once per accepted connection, so each connection
    /// gets independent queues, rate limiter state, and dead-letter routing
    /// derived from the same configuration. A dead-letter queue configured
    /// here receives codec frames, matching ADR 009.
    ///
    /// The template is validated when the application is prepared, so an
    /// invalid capacity or rate fails before any connection is accepted.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use bytes::Bytes;
    /// use tokio::sync::mpsc;
    /// use wireframe::{app::WireframeApp, push::PushQueues};
    ///
    /// # fn build() -> WireframeApp {
    /// #     WireframeApp::new().expect("builder creation should not fail")
    /// # }
    /// # fn main() {
    /// let (dlq_tx, _dlq_rx) = mpsc::channel(16);
    /// let templates = PushQueues::<Bytes>::builder()
    ///     .high_capacity(8)
    ///     .low_capacity(8)
    ///     .dlq(Some(dlq_tx));
    /// let app = build().with_push_queues(templates);
    /// # let _ = app;
    /// # }
    /// ```
    #[must_use]
    pub fn with_push_queues(mut self, push_queues: PushQueuesBuilder<F::Frame>) -> Self {
        self.push_queues = Some(push_queues);
        self
    }

    /// Share a session registry across every connection.
    ///
    /// Connections register their push handle with this registry on setup and
    /// deregister it on close, so application code holding the same `Arc` can
    /// address individual connections. Without this call the application owns
    /// a private registry that is still populated but not reachable by the
    /// caller.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use std::sync::Arc;
    ///
    /// use bytes::Bytes;
    /// use wireframe::{app::WireframeApp, session::SessionRegistry};
    ///
    /// let registry = Arc::new(SessionRegistry::<Bytes>::new());
    /// let app = WireframeApp::new()
    ///     .expect("builder creation should not fail")
    ///     .with_session_registry(Arc::clone(&registry));
    /// # let _ = app;
    /// ```
    #[must_use]
    pub fn with_session_registry(mut self, registry: Arc<SessionRegistry<F::Frame>>) -> Self {
        self.session_registry = registry;
        self
    }

    /// Return the registry every connection built from this application uses.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use std::sync::Arc;
    ///
    /// use bytes::Bytes;
    /// use wireframe::{app::WireframeApp, session::SessionRegistry};
    ///
    /// let registry = Arc::new(SessionRegistry::<Bytes>::new());
    /// let app = WireframeApp::new()
    ///     .expect("builder creation should not fail")
    ///     .with_session_registry(Arc::clone(&registry));
    /// assert!(Arc::ptr_eq(&app.session_registry(), &registry));
    /// ```
    #[must_use]
    pub fn session_registry(&self) -> Arc<SessionRegistry<F::Frame>> {
        Arc::clone(&self.session_registry)
    }

    /// Configure a legacy Dead Letter Queue for dropped push frames.
    ///
    /// # Deprecated
    ///
    /// This sink receives serialized payloads and is no longer consulted when
    /// building connection queues. Use
    /// [`with_push_queues`][Self::with_push_queues] with
    /// [`PushQueues::builder`][crate::push::PushQueues::builder] and its
    /// `dlq` method so dropped frames arrive as codec frames, as required by
    /// ADR 009.
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
    #[deprecated(
        since = "0.4.0",
        note = "use with_push_queues(PushQueues::builder().dlq(sender)) so dropped frames arrive \
                as codec frames"
    )]
    pub fn with_push_dlq(mut self, dlq: tokio::sync::mpsc::Sender<Vec<u8>>) -> Self {
        self.push_dlq = Some(dlq);
        self
    }
}

#[cfg(test)]
#[path = "push_tests.rs"]
mod tests;
