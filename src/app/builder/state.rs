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
    /// Store a shared state value accessible to request extractors.
    ///
    /// The value can later be retrieved using [`crate::extractor::SharedState`]. Registering
    /// another value of the same type overwrites the previous one.
    #[must_use]
    pub fn app_data<T>(self, state: T) -> Self
    where
        T: Send + Sync + 'static,
    {
        self.app_data.insert(state);
        self
    }
}
