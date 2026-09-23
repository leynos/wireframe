//! Compact timing switches for the six client operation categories.

/// Client operation whose elapsed time can be recorded.
#[derive(Clone, Copy, Debug)]
pub(crate) enum ClientOperation {
    /// Connection establishment.
    Connect,
    /// One-way send.
    Send,
    /// One-way receive.
    Receive,
    /// Request-response call.
    Call,
    /// Streaming call.
    Streaming,
    /// Orderly connection close.
    Close,
}

impl ClientOperation {
    /// Return the independent bit reserved for this operation.
    const fn mask(self) -> u8 {
        match self {
            Self::Connect => 0b00_0001,
            Self::Send => 0b00_0010,
            Self::Receive => 0b00_0100,
            Self::Call => 0b00_1000,
            Self::Streaming => 0b01_0000,
            Self::Close => 0b10_0000,
        }
    }
}

/// Timing selection stored as one value rather than six independent booleans.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ClientTimingFlags(u8);

impl ClientTimingFlags {
    /// Set one operation without changing the other five.
    pub(crate) const fn with_enabled(mut self, operation: ClientOperation, enabled: bool) -> Self {
        let mask = operation.mask();
        if enabled {
            self.0 |= mask;
        } else {
            self.0 &= !mask;
        }
        self
    }

    /// Set every operation to the same timing policy.
    pub(crate) const fn all(enabled: bool) -> Self { Self(if enabled { 0b11_1111 } else { 0 }) }

    /// Report whether timing is enabled for one operation.
    pub(crate) const fn is_enabled(self, operation: ClientOperation) -> bool {
        self.0 & operation.mask() != 0
    }
}
