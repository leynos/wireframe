//! Shared state configuration for `WireframeApp`.

use super::WireframeApp;
use crate::{app::Packet, codec::FrameCodec, serializer::Serializer};

impl<S, C, E, F> WireframeApp<S, C, E, F>
where
    S: Serializer + Send + Sync,
    C: Send + 'static,
    E: Packet,
    F: FrameCodec,
{
    /// Store a shared state value for the application.
    ///
    /// Registering another value of the same type overwrites the previous one.
    /// The connection runtime does not pass these values to route handlers or
    /// middleware: [`crate::extractor::SharedState`] reads from a
    /// [`crate::extractor::MessageRequest`] that the caller populates, so a
    /// handler that needs shared state should capture it in its closure.
    #[must_use]
    pub fn app_data<T>(self, state: T) -> Self
    where
        T: Send + Sync + 'static,
    {
        self.app_data.insert(state);
        self
    }
}
