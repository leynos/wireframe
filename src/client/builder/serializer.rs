//! Serializer configuration methods for `WireframeClientBuilder`.

use super::WireframeClientBuilder;
use crate::serializer::Serializer;

impl<S, P, C> WireframeClientBuilder<S, P, C>
where
    S: Serializer + Send + Sync,
{
    /// Replace the serializer used for encoding and decoding messages.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::{BincodeSerializer, client::WireframeClientBuilder};
    ///
    /// let builder = WireframeClientBuilder::new().serializer(BincodeSerializer);
    /// let _ = builder;
    /// ```
    #[must_use]
    pub fn serializer<Ser>(self, serializer: Ser) -> WireframeClientBuilder<Ser, P, C>
    where
        Ser: Serializer + Send + Sync,
    {
        builder_field_update!(self, serializer = serializer)
    }
}
