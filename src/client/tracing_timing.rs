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

#[cfg(test)]
mod tests {
    //! Checks timing updates against a six-operation reference model.

    use proptest::{collection::vec, prelude::*};

    use super::{ClientOperation, ClientTimingFlags};
    use crate::client::TracingConfig;

    const OPERATIONS: [ClientOperation; 6] = [
        ClientOperation::Connect,
        ClientOperation::Send,
        ClientOperation::Receive,
        ClientOperation::Call,
        ClientOperation::Streaming,
        ClientOperation::Close,
    ];

    #[derive(Clone, Copy, Debug)]
    enum TimingUpdate {
        Set(ClientOperation, bool),
        SetAll(bool),
    }

    fn timing_update_strategy() -> impl Strategy<Value = TimingUpdate> {
        let operation = prop_oneof![
            Just(ClientOperation::Connect),
            Just(ClientOperation::Send),
            Just(ClientOperation::Receive),
            Just(ClientOperation::Call),
            Just(ClientOperation::Streaming),
            Just(ClientOperation::Close),
        ];

        prop_oneof![
            (operation, any::<bool>())
                .prop_map(|(operation, enabled)| TimingUpdate::Set(operation, enabled)),
            any::<bool>().prop_map(TimingUpdate::SetAll),
        ]
    }

    fn set_expected_timing(expected: &mut [bool; 6], operation: ClientOperation, enabled: bool) {
        let [connect, send, receive, call, streaming, close] = expected;
        let selected = match operation {
            ClientOperation::Connect => connect,
            ClientOperation::Send => send,
            ClientOperation::Receive => receive,
            ClientOperation::Call => call,
            ClientOperation::Streaming => streaming,
            ClientOperation::Close => close,
        };
        *selected = enabled;
    }

    fn set_config_timing(
        config: TracingConfig,
        operation: ClientOperation,
        enabled: bool,
    ) -> TracingConfig {
        match operation {
            ClientOperation::Connect => config.with_connect_timing(enabled),
            ClientOperation::Send => config.with_send_timing(enabled),
            ClientOperation::Receive => config.with_receive_timing(enabled),
            ClientOperation::Call => config.with_call_timing(enabled),
            ClientOperation::Streaming => config.with_streaming_timing(enabled),
            ClientOperation::Close => config.with_close_timing(enabled),
        }
    }

    fn flags_snapshot(flags: ClientTimingFlags) -> [bool; 6] {
        OPERATIONS.map(|operation| flags.is_enabled(operation))
    }

    fn config_snapshot(config: &TracingConfig) -> [bool; 6] {
        OPERATIONS.map(|operation| config.timing_enabled(operation))
    }

    proptest! {
        #[test]
        fn timing_updates_match_reference_after_every_action(
            updates in vec(timing_update_strategy(), 1..64),
        ) {
            let mut flags = ClientTimingFlags::default();
            let mut config = TracingConfig::default();
            let mut expected = [false; 6];

            for (step, update) in updates.into_iter().enumerate() {
                match update {
                    TimingUpdate::Set(operation, enabled) => {
                        flags = flags.with_enabled(operation, enabled);
                        config = set_config_timing(config, operation, enabled);
                        set_expected_timing(&mut expected, operation, enabled);
                    }
                    TimingUpdate::SetAll(enabled) => {
                        flags = ClientTimingFlags::all(enabled);
                        config = config.with_all_timing(enabled);
                        expected = [enabled; 6];
                    }
                }

                prop_assert_eq!(
                    flags_snapshot(flags),
                    expected,
                    "ClientTimingFlags diverged after update {}: {:?}",
                    step,
                    update,
                );
                prop_assert_eq!(
                    config_snapshot(&config),
                    expected,
                    "TracingConfig diverged after update {}: {:?}",
                    step,
                    update,
                );
            }
        }
    }
}
