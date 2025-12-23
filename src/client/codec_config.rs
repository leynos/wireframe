//! Codec configuration for wireframe clients.

use tokio_util::codec::LengthDelimitedCodec;

use crate::frame::{Endianness, LengthFormat};

const MIN_FRAME_LENGTH: usize = 64;
const MAX_FRAME_LENGTH: usize = 16 * 1024 * 1024;
const DEFAULT_MAX_FRAME_LENGTH: usize = 1024;

/// Codec configuration for the wireframe client.
///
/// # Examples
///
/// ```
/// use wireframe::client::ClientCodecConfig;
///
/// let codec = ClientCodecConfig::default().max_frame_length(2048);
/// assert_eq!(codec.max_frame_length_value(), 2048);
/// ```
#[derive(Clone, Copy, Debug)]
pub struct ClientCodecConfig {
    length_format: LengthFormat,
    max_frame_length: usize,
}

impl Default for ClientCodecConfig {
    fn default() -> Self {
        Self {
            length_format: LengthFormat::default(),
            max_frame_length: DEFAULT_MAX_FRAME_LENGTH,
        }
    }
}

impl ClientCodecConfig {
    /// Set the maximum frame length for encoding and decoding.
    ///
    /// The value is clamped between 64 bytes and 16 MiB.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::client::ClientCodecConfig;
    ///
    /// let codec = ClientCodecConfig::default().max_frame_length(2048);
    /// assert_eq!(codec.max_frame_length_value(), 2048);
    /// ```
    #[must_use]
    pub fn max_frame_length(mut self, max_frame_length: usize) -> Self {
        self.max_frame_length = max_frame_length.clamp(MIN_FRAME_LENGTH, MAX_FRAME_LENGTH);
        self
    }

    /// Set the length prefix format used by the codec.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::{
    ///     client::ClientCodecConfig,
    ///     frame::{Endianness, LengthFormat},
    /// };
    ///
    /// let codec = ClientCodecConfig::default().length_format(LengthFormat::u16_le());
    /// assert_eq!(codec.length_format_value().bytes(), 2);
    /// assert_eq!(codec.length_format_value().endianness(), Endianness::Little);
    /// ```
    #[must_use]
    pub fn length_format(mut self, length_format: LengthFormat) -> Self {
        self.length_format = length_format;
        self
    }

    /// Return the configured maximum frame length.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::client::ClientCodecConfig;
    ///
    /// let codec = ClientCodecConfig::default();
    /// assert_eq!(codec.max_frame_length_value(), 1024);
    /// ```
    #[must_use]
    pub const fn max_frame_length_value(&self) -> usize { self.max_frame_length }

    /// Return the configured length prefix format.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::{client::ClientCodecConfig, frame::Endianness};
    ///
    /// let codec = ClientCodecConfig::default();
    /// assert_eq!(codec.length_format_value().bytes(), 4);
    /// assert_eq!(codec.length_format_value().endianness(), Endianness::Big);
    /// ```
    #[must_use]
    pub const fn length_format_value(&self) -> LengthFormat { self.length_format }

    pub(crate) fn build_codec(&self) -> LengthDelimitedCodec {
        let mut builder = LengthDelimitedCodec::builder();
        builder.length_field_length(self.length_format.bytes());
        match self.length_format.endianness() {
            Endianness::Big => {
                builder.big_endian();
            }
            Endianness::Little => {
                builder.little_endian();
            }
        }
        builder.max_frame_length(self.max_frame_length);
        builder.new_codec()
    }
}
