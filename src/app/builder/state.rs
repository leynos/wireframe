//! Shared state configuration for `WireframeApp`.

use std::{any::TypeId, sync::Arc};

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
    pub fn app_data<T>(mut self, state: T) -> Self
    where
        T: Send + Sync + 'static,
    {
        self.app_data.insert(
            TypeId::of::<T>(),
            Arc::new(state) as Arc<dyn std::any::Any + Send + Sync>,
        );
        self
    }
}
