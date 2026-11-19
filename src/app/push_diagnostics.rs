//! Structured diagnostics for push writes the connection abandons.
//!
//! A push write the app owns is bounded by the connection's stop token and, once
//! the peer can no longer be observed, by the idle grace. When either bound
//! abandons the write, the frame is dropped without an error reaching anyone,
//! so the abandonment is reported here: one structured event naming the
//! connection, and one bounded counter an operator can alert on.
//!
//! The event carries identifiers and a duration only. Frame payloads are never
//! logged, and the counter labels stay a fixed vocabulary so peer addresses and
//! connection identifiers cannot create new series.

use std::{net::SocketAddr, time::Duration};

use tracing::warn;

use crate::{metrics::PushWriteAbandonReason, push::PushPriority, session::ConnectionId};

/// Connection context attached to an abandoned push write.
///
/// Every field is copyable and the push priority is optional, so a writer that
/// drains both lanes in one sequence can report `"mixed"` rather than inventing
/// a priority the batch does not have.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PushWriteDiagnostics {
    /// Identifier the server allocated for the connection, when it has one.
    pub(crate) connection_id: Option<ConnectionId>,
    /// Peer address of the accepted socket, when the transport reported one.
    pub(crate) peer_addr: Option<SocketAddr>,
    /// Lane the batch was taken from; absent when both lanes were drained.
    pub(crate) priority: Option<PushPriority>,
}

impl PushWriteDiagnostics {
    /// Build diagnostics for a write whose batch priority is not yet known.
    pub(crate) const fn new(
        connection_id: Option<ConnectionId>,
        peer_addr: Option<SocketAddr>,
    ) -> Self {
        Self {
            connection_id,
            peer_addr,
            priority: None,
        }
    }

    /// Attach the priority lane the batch being written was taken from.
    pub(crate) const fn with_priority(self, priority: Option<PushPriority>) -> Self {
        Self { priority, ..self }
    }

    /// Report one abandoned push write.
    ///
    /// An exhausted idle grace is the case an operator needs to see: the peer
    /// stopped reading while still connected, so the connection ended because
    /// of it. A shutdown abandonment is counted but not warned about, because
    /// every connection that stops with queued output would produce one and the
    /// connection's own teardown logs already record the stop.
    pub(crate) fn record_abandonment(self, reason: PushWriteAbandonReason, waited: Duration) {
        if reason == PushWriteAbandonReason::IdleGraceExpired {
            warn!(
                connection_id = ?self.connection_id,
                peer_addr = ?self.peer_addr,
                priority = priority_label(self.priority),
                reason = reason.as_str(),
                elapsed_ms = elapsed_ms(waited),
                "abandoning a push write: the peer stopped reading"
            );
        }
        reason.record();
    }
}

/// Stable label naming the lane a push batch came from.
const fn priority_label(priority: Option<PushPriority>) -> &'static str {
    match priority {
        Some(PushPriority::High) => "high",
        Some(PushPriority::Low) => "low",
        None => "mixed",
    }
}

/// Whole milliseconds waited, saturating rather than wrapping.
fn elapsed_ms(waited: Duration) -> u64 { u64::try_from(waited.as_millis()).unwrap_or(u64::MAX) }

#[cfg(test)]
mod tests {
    //! Tests pinning the operator-visible abandonment diagnostics.
    //!
    //! An exhausted idle grace is the one per-connection signal an operator
    //! gets that a peer stopped reading, so the fields and vocabulary of its
    //! warning are fixed here. The shutdown path deliberately stays silent;
    //! pinning that silence stops a later edit from producing one warning per
    //! connection during ordinary teardown.

    use std::{net::SocketAddr, time::Duration};

    use wireframe_testing::logger;

    use super::PushWriteDiagnostics;
    use crate::{metrics::PushWriteAbandonReason, push::PushPriority, session::ConnectionId};

    /// An exhausted idle grace warns once, naming the connection it dropped.
    ///
    /// The warning is the only trace of a write the peer never read, so it
    /// must carry the connection identity, peer, lane and wait an operator
    /// needs to identify the stalled connection.
    #[test]
    fn idle_grace_abandonment_warns_with_the_connection_identity() {
        let connection_id = ConnectionId::new(4_242);
        let peer = SocketAddr::from(([127, 0, 0, 1], 9_042));
        let mut log = logger();
        log.clear();

        PushWriteDiagnostics::new(Some(connection_id), Some(peer))
            .with_priority(Some(PushPriority::High))
            .record_abandonment(
                PushWriteAbandonReason::IdleGraceExpired,
                Duration::from_millis(1_234),
            );

        // The log buffer is process-wide, so only records naming this
        // connection and peer count as this test's abandonment.
        let warnings: Vec<String> = std::iter::from_fn(|| log.pop())
            .map(|record| record.args().to_owned())
            .filter(|message| {
                message.contains("ConnectionId(4242)") && message.contains(&peer.to_string())
            })
            .collect();
        assert_eq!(
            warnings.len(),
            1,
            "exactly one abandonment warning must be captured; captured: {warnings:#?}"
        );

        let warning = warnings
            .first()
            .expect("the single captured warning must be inspectable");
        let peer_field = format!("peer_addr=Some({peer})");
        // The log bridge renders string fields with `Debug`, so their quotes
        // are part of the captured line.
        for fragment in [
            "ConnectionId(4242)",
            peer_field.as_str(),
            r#"priority="high""#,
            r#"reason="idle_grace_expired""#,
            "elapsed_ms=1234",
        ] {
            assert!(
                warning.contains(fragment),
                "the abandonment warning must name {fragment}; captured: {warning}"
            );
        }
    }

    /// A shutdown abandonment is counted but emits no warning.
    ///
    /// Every teardown with queued output abandons its writes, so a warning
    /// here would flood the log during ordinary shutdown while adding nothing
    /// beyond the connection's own stop diagnostics.
    #[test]
    fn shutdown_abandonment_does_not_warn() {
        let mut log = logger();
        log.clear();

        PushWriteDiagnostics::new(
            Some(ConnectionId::new(4_243)),
            Some(SocketAddr::from(([127, 0, 0, 1], 9_043))),
        )
        .with_priority(Some(PushPriority::Low))
        .record_abandonment(PushWriteAbandonReason::Shutdown, Duration::from_millis(10));

        let warnings: Vec<String> = std::iter::from_fn(|| log.pop())
            .map(|record| record.args().to_owned())
            .filter(|message| message.contains("abandoning a push write"))
            .collect();
        assert!(
            warnings.is_empty(),
            "a shutdown abandonment must not warn; captured: {warnings:#?}"
        );
    }
}
