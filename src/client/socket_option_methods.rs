//! Socket option builder methods for `WireframeClientBuilder`.
//!
//! This module contains convenience methods that configure socket options
//! without changing the generic type parameters. These methods delegate
//! to `SocketOptions` methods internally.

use std::time::Duration;

use super::{SocketOptions, WireframeClientBuilder};
use crate::serializer::Serializer;

impl<S, P, C> WireframeClientBuilder<S, P, C>
where
    S: Serializer + Send + Sync,
{
    /// Replace the socket options applied before connecting.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::client::{SocketOptions, WireframeClientBuilder};
    ///
    /// let options = SocketOptions::default().nodelay(true);
    /// let builder = WireframeClientBuilder::new().socket_options(options);
    /// let _ = builder;
    /// ```
    #[must_use]
    pub fn socket_options(mut self, socket_options: SocketOptions) -> Self {
        self.socket_options = socket_options;
        self
    }

    /// Configure `TCP_NODELAY` for the connection.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::client::WireframeClientBuilder;
    ///
    /// let builder = WireframeClientBuilder::new().nodelay(true);
    /// let _ = builder;
    /// ```
    #[must_use]
    pub fn nodelay(mut self, enabled: bool) -> Self {
        self.socket_options = self.socket_options.nodelay(enabled);
        self
    }

    /// Configure `SO_KEEPALIVE` for the connection.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    ///
    /// use wireframe::client::WireframeClientBuilder;
    ///
    /// let builder = WireframeClientBuilder::new().keepalive(Some(Duration::from_secs(30)));
    /// let _ = builder;
    /// ```
    #[must_use]
    pub fn keepalive(mut self, duration: Option<Duration>) -> Self {
        self.socket_options = self.socket_options.keepalive(duration);
        self
    }

    /// Configure TCP linger behaviour for the connection.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    ///
    /// use wireframe::client::WireframeClientBuilder;
    ///
    /// let builder = WireframeClientBuilder::new().linger(Some(Duration::from_secs(1)));
    /// let _ = builder;
    /// ```
    #[must_use]
    pub fn linger(mut self, duration: Option<Duration>) -> Self {
        self.socket_options = self.socket_options.linger(duration);
        self
    }

    /// Configure the socket send buffer size.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::client::WireframeClientBuilder;
    ///
    /// let builder = WireframeClientBuilder::new().send_buffer_size(4096);
    /// let _ = builder;
    /// ```
    #[must_use]
    pub fn send_buffer_size(mut self, size: u32) -> Self {
        self.socket_options = self.socket_options.send_buffer_size(size);
        self
    }

    /// Configure the socket receive buffer size.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::client::WireframeClientBuilder;
    ///
    /// let builder = WireframeClientBuilder::new().recv_buffer_size(4096);
    /// let _ = builder;
    /// ```
    #[must_use]
    pub fn recv_buffer_size(mut self, size: u32) -> Self {
        self.socket_options = self.socket_options.recv_buffer_size(size);
        self
    }

    /// Configure `SO_REUSEADDR` for the connection.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::client::WireframeClientBuilder;
    ///
    /// let builder = WireframeClientBuilder::new().reuseaddr(true);
    /// let _ = builder;
    /// ```
    #[must_use]
    pub fn reuseaddr(mut self, enabled: bool) -> Self {
        self.socket_options = self.socket_options.reuseaddr(enabled);
        self
    }

    /// Configure `SO_REUSEPORT` for the connection on supported platforms.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::client::WireframeClientBuilder;
    ///
    /// let builder = WireframeClientBuilder::new().reuseport(true);
    /// let _ = builder;
    /// ```
    #[cfg(all(
        unix,
        not(target_os = "solaris"),
        not(target_os = "illumos"),
        not(target_os = "cygwin"),
    ))]
    #[must_use]
    pub fn reuseport(mut self, enabled: bool) -> Self {
        self.socket_options = self.socket_options.reuseport(enabled);
        self
    }
}
