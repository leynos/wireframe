//! Codec configuration methods for `WireframeClientBuilder`.

use super::WireframeClientBuilder;
use crate::{client::ClientCodecConfig, frame::LengthFormat, serializer::Serializer};

impl<S, P, C> WireframeClientBuilder<S, P, C>
where
    S: Serializer + Send + Sync,
{
    /// Configure codec settings for the connection.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::client::{ClientCodecConfig, WireframeClientBuilder};
    ///
    /// let codec = ClientCodecConfig::default().max_frame_length(2048);
    /// let builder = WireframeClientBuilder::new().codec_config(codec);
    /// let _ = builder;
    /// ```
    #[must_use]
    pub fn codec_config(mut self, codec_config: ClientCodecConfig) -> Self {
        self.codec_config = codec_config;
        self
    }

    /// Configure the maximum frame length for the connection.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::client::WireframeClientBuilder;
    ///
    /// let builder = WireframeClientBuilder::new().max_frame_length(2048);
    /// let _ = builder;
    /// ```
    #[must_use]
    pub fn max_frame_length(mut self, max_frame_length: usize) -> Self {
        self.codec_config = self.codec_config.max_frame_length(max_frame_length);
        self
    }

    /// Configure the length prefix format for the connection.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::{client::WireframeClientBuilder, frame::LengthFormat};
    ///
    /// let builder = WireframeClientBuilder::new().length_format(LengthFormat::u16_be());
    /// let _ = builder;
    /// ```
    #[must_use]
    pub fn length_format(mut self, length_format: LengthFormat) -> Self {
        self.codec_config = self.codec_config.length_format(length_format);
        self
    }
}
