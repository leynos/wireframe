//! Socket options for wireframe clients.

use std::{io, time::Duration};

use socket2::{SockRef, TcpKeepalive};
use tokio::net::TcpSocket;

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
    /// Whether small writes bypass Nagle coalescing.
    nodelay: Option<bool>,
    /// Keepalive policy; `None` means leave the platform default untouched.
    keepalive: Option<KeepAliveSetting>,
    /// Requested kernel send-buffer capacity.
    send_buffer_size: Option<u32>,
    /// Requested kernel receive-buffer capacity.
    recv_buffer_size: Option<u32>,
    /// Whether the address may be rebound while it is in use.
    reuseaddr: Option<bool>,
    #[cfg(all(
        unix,
        not(target_os = "solaris"),
        not(target_os = "illumos"),
        not(target_os = "cygwin"),
    ))]
    /// Whether supported platforms may share a listening port.
    reuseport: Option<bool>,
}

/// Internal representation of the optional TCP keepalive override.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum KeepAliveSetting {
    /// Explicitly disable keepalive probes.
    Disabled,
    /// Enable probes after the supplied idle duration.
    Duration(Duration),
}

impl KeepAliveSetting {
    /// Convert the policy to the socket API's optional duration form.
    const fn to_option(self) -> Option<Duration> {
        match self {
            Self::Disabled => None,
            Self::Duration(value) => Some(value),
        }
    }
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

    /// Apply every configured option, preserving the first OS error.
    pub(crate) fn apply(&self, socket: &TcpSocket) -> io::Result<()> {
        self.apply_nodelay(socket)?;
        self.apply_keepalive(socket)?;
        self.apply_send_buffer_size(socket)?;
        self.apply_recv_buffer_size(socket)?;
        self.apply_reuseaddr(socket)?;
        self.apply_reuseport(socket)?;
        Ok(())
    }

    /// Apply `TCP_NODELAY` when the caller supplied an override.
    fn apply_nodelay(&self, socket: &TcpSocket) -> io::Result<()> {
        if let Some(enabled) = self.nodelay {
            socket.set_nodelay(enabled)?;
        }
        Ok(())
    }

    /// Apply keepalive enablement and its platform-specific idle interval.
    fn apply_keepalive(&self, socket: &TcpSocket) -> io::Result<()> {
        if let Some(keepalive) = self.keepalive {
            match keepalive.to_option() {
                Some(duration) => {
                    socket.set_keepalive(true)?;
                    let sock_ref = SockRef::from(socket);
                    let config = TcpKeepalive::new().with_time(duration);
                    sock_ref.set_tcp_keepalive(&config)?;
                }
                None => {
                    socket.set_keepalive(false)?;
                }
            }
        }
        Ok(())
    }

    /// Apply the requested kernel send-buffer capacity.
    fn apply_send_buffer_size(&self, socket: &TcpSocket) -> io::Result<()> {
        if let Some(size) = self.send_buffer_size {
            socket.set_send_buffer_size(size)?;
        }
        Ok(())
    }

    /// Apply the requested kernel receive-buffer capacity.
    fn apply_recv_buffer_size(&self, socket: &TcpSocket) -> io::Result<()> {
        if let Some(size) = self.recv_buffer_size {
            socket.set_recv_buffer_size(size)?;
        }
        Ok(())
    }

    /// Apply address reuse before the connect attempt.
    fn apply_reuseaddr(&self, socket: &TcpSocket) -> io::Result<()> {
        if let Some(enabled) = self.reuseaddr {
            socket.set_reuseaddr(enabled)?;
        }
        Ok(())
    }

    #[cfg(all(
        unix,
        not(target_os = "solaris"),
        not(target_os = "illumos"),
        not(target_os = "cygwin"),
    ))]
    /// Apply port sharing on platforms that expose this option.
    fn apply_reuseport(&self, socket: &TcpSocket) -> io::Result<()> {
        if let Some(enabled) = self.reuseport {
            socket.set_reuseport(enabled)?;
        }
        Ok(())
    }

    #[cfg(not(all(
        unix,
        not(target_os = "solaris"),
        not(target_os = "illumos"),
        not(target_os = "cygwin"),
    )))]
    /// Keep the option pipeline portable where port sharing is unavailable.
    fn apply_reuseport(&self, _socket: &TcpSocket) -> io::Result<()> { Ok(()) }
}
