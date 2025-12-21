//! Codec and socket configuration for wireframe clients.

use std::{io, time::Duration};

use socket2::{SockRef, TcpKeepalive};
use tokio::net::TcpSocket;
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

/// Socket options applied before connecting a client.
///
/// # Examples
///
/// ```
/// use std::time::Duration;
///
/// use wireframe::client::SocketOptions;
///
/// let options = SocketOptions::default()
///     .nodelay(true)
///     .keepalive(Some(Duration::from_secs(30)));
/// let expected = SocketOptions::default()
///     .nodelay(true)
///     .keepalive(Some(Duration::from_secs(30)));
/// assert_eq!(options, expected);
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SocketOptions {
    nodelay: Option<bool>,
    keepalive: Option<KeepAliveSetting>,
    linger: Option<LingerSetting>,
    send_buffer_size: Option<u32>,
    recv_buffer_size: Option<u32>,
    reuseaddr: Option<bool>,
    #[cfg(all(
        unix,
        not(target_os = "solaris"),
        not(target_os = "illumos"),
        not(target_os = "cygwin"),
    ))]
    reuseport: Option<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LingerSetting {
    Disabled,
    Duration(Duration),
}

impl LingerSetting {
    const fn to_option(self) -> Option<Duration> {
        match self {
            Self::Disabled => None,
            Self::Duration(value) => Some(value),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum KeepAliveSetting {
    Disabled,
    Duration(Duration),
}

impl SocketOptions {
    /// Configure `TCP_NODELAY` behaviour on the socket.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::client::SocketOptions;
    ///
    /// let options = SocketOptions::default().nodelay(true);
    /// let expected = SocketOptions::default().nodelay(true);
    /// assert_eq!(options, expected);
    /// ```
    #[must_use]
    pub fn nodelay(mut self, enabled: bool) -> Self {
        self.nodelay = Some(enabled);
        self
    }

    /// Configure `SO_KEEPALIVE` behaviour on the socket.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    ///
    /// use wireframe::client::SocketOptions;
    ///
    /// let options = SocketOptions::default().keepalive(Some(Duration::from_secs(30)));
    /// let expected = SocketOptions::default().keepalive(Some(Duration::from_secs(30)));
    /// assert_eq!(options, expected);
    /// ```
    #[must_use]
    pub fn keepalive(mut self, duration: Option<Duration>) -> Self {
        self.keepalive = Some(match duration {
            Some(value) => KeepAliveSetting::Duration(value),
            None => KeepAliveSetting::Disabled,
        });
        self
    }

    /// Configure TCP linger settings on the socket.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    ///
    /// use wireframe::client::SocketOptions;
    ///
    /// let options = SocketOptions::default().linger(Some(Duration::from_secs(1)));
    /// let expected = SocketOptions::default().linger(Some(Duration::from_secs(1)));
    /// assert_eq!(options, expected);
    /// ```
    #[must_use]
    pub fn linger(mut self, duration: Option<Duration>) -> Self {
        self.linger = Some(match duration {
            Some(value) => LingerSetting::Duration(value),
            None => LingerSetting::Disabled,
        });
        self
    }

    /// Configure the socket send buffer size.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::client::SocketOptions;
    ///
    /// let options = SocketOptions::default().send_buffer_size(4096);
    /// let expected = SocketOptions::default().send_buffer_size(4096);
    /// assert_eq!(options, expected);
    /// ```
    #[must_use]
    pub fn send_buffer_size(mut self, size: u32) -> Self {
        self.send_buffer_size = Some(size);
        self
    }

    /// Configure the socket receive buffer size.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::client::SocketOptions;
    ///
    /// let options = SocketOptions::default().recv_buffer_size(4096);
    /// let expected = SocketOptions::default().recv_buffer_size(4096);
    /// assert_eq!(options, expected);
    /// ```
    #[must_use]
    pub fn recv_buffer_size(mut self, size: u32) -> Self {
        self.recv_buffer_size = Some(size);
        self
    }

    /// Configure `SO_REUSEADDR` behaviour on the socket.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::client::SocketOptions;
    ///
    /// let options = SocketOptions::default().reuseaddr(true);
    /// let expected = SocketOptions::default().reuseaddr(true);
    /// assert_eq!(options, expected);
    /// ```
    #[must_use]
    pub fn reuseaddr(mut self, enabled: bool) -> Self {
        self.reuseaddr = Some(enabled);
        self
    }

    /// Configure `SO_REUSEPORT` behaviour on supported platforms.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::client::SocketOptions;
    ///
    /// let options = SocketOptions::default().reuseport(true);
    /// let expected = SocketOptions::default().reuseport(true);
    /// assert_eq!(options, expected);
    /// ```
    #[cfg(all(
        unix,
        not(target_os = "solaris"),
        not(target_os = "illumos"),
        not(target_os = "cygwin"),
    ))]
    #[must_use]
    pub fn reuseport(mut self, enabled: bool) -> Self {
        self.reuseport = Some(enabled);
        self
    }

    pub(crate) fn apply(&self, socket: &TcpSocket) -> io::Result<()> {
        if let Some(enabled) = self.nodelay {
            socket.set_nodelay(enabled)?;
        }
        if let Some(keepalive) = self.keepalive {
            match keepalive {
                KeepAliveSetting::Disabled => {
                    socket.set_keepalive(false)?;
                }
                KeepAliveSetting::Duration(duration) => {
                    socket.set_keepalive(true)?;
                    let sock_ref = SockRef::from(socket);
                    let config = TcpKeepalive::new().with_time(duration);
                    sock_ref.set_tcp_keepalive(&config)?;
                }
            }
        }
        if let Some(linger) = self.linger {
            socket.set_linger(linger.to_option())?;
        }
        if let Some(size) = self.send_buffer_size {
            socket.set_send_buffer_size(size)?;
        }
        if let Some(size) = self.recv_buffer_size {
            socket.set_recv_buffer_size(size)?;
        }
        if let Some(enabled) = self.reuseaddr {
            socket.set_reuseaddr(enabled)?;
        }
        #[cfg(all(
            unix,
            not(target_os = "solaris"),
            not(target_os = "illumos"),
            not(target_os = "cygwin"),
        ))]
        if let Some(enabled) = self.reuseport {
            socket.set_reuseport(enabled)?;
        }
        Ok(())
    }
}
