//! Tracing span and event helpers for wireframe client operations.
//!
//! These helpers centralise span creation with dynamic level selection and
//! per-command timing emission, keeping the instrumentation logic out of the
//! hot-path client methods.

use std::time::Instant;

use tracing::{Level, Span};

use super::tracing_config::TracingConfig;

/// Create a tracing span at a dynamically selected level.
///
/// The level is matched against the five `tracing::Level` variants. Each
/// branch calls the corresponding `tracing::<level>_span!` macro, which
/// ensures the span metadata is statically known per branch while the
/// branch selection is dynamic.
macro_rules! dynamic_span {
    ($level:expr, $name:expr $(, $($field:tt)*)?) => {
        match $level {
            Level::ERROR => tracing::error_span!($name $(, $($field)*)?),
            Level::WARN  => tracing::warn_span!($name $(, $($field)*)?),
            Level::INFO  => tracing::info_span!($name $(, $($field)*)?),
            Level::DEBUG => tracing::debug_span!($name $(, $($field)*)?),
            Level::TRACE => tracing::trace_span!($name $(, $($field)*)?),
        }
    };
}

/// Create a span for the `connect` operation.
#[expect(
    clippy::cognitive_complexity,
    reason = "complexity from dynamic_span! macro expansion — five match arms are inherent"
)]
pub(crate) fn connect_span(config: &TracingConfig, peer_addr: &str) -> Span {
    dynamic_span!(
        config.connect_level,
        "client.connect",
        peer.addr = peer_addr
    )
}

/// Create a span for the `send` operation.
#[expect(
    clippy::cognitive_complexity,
    reason = "complexity from dynamic_span! macro expansion"
)]
pub(crate) fn send_span(config: &TracingConfig, frame_bytes: usize) -> Span {
    dynamic_span!(config.send_level, "client.send", frame.bytes = frame_bytes)
}

/// Create a span for the `receive` operation.
///
/// The `frame.bytes` and `result` fields are recorded after the frame arrives
/// using [`Span::record`].
#[expect(
    clippy::cognitive_complexity,
    reason = "complexity from dynamic_span! macro expansion"
)]
pub(crate) fn receive_span(config: &TracingConfig) -> Span {
    dynamic_span!(
        config.receive_level,
        "client.receive",
        frame.bytes = tracing::field::Empty,
        result = tracing::field::Empty
    )
}

/// Create a span for the `send_envelope` operation.
#[expect(
    clippy::cognitive_complexity,
    reason = "complexity from dynamic_span! macro expansion"
)]
pub(crate) fn send_envelope_span(
    config: &TracingConfig,
    correlation_id: u64,
    frame_bytes: usize,
) -> Span {
    dynamic_span!(
        config.send_level,
        "client.send_envelope",
        correlation_id = correlation_id,
        frame.bytes = frame_bytes
    )
}

/// Create a span for the `call` operation.
///
/// The `result` field is recorded when the call completes using
/// [`Span::record`].
#[expect(
    clippy::cognitive_complexity,
    reason = "complexity from dynamic_span! macro expansion"
)]
pub(crate) fn call_span(config: &TracingConfig) -> Span {
    dynamic_span!(
        config.call_level,
        "client.call",
        result = tracing::field::Empty
    )
}

/// Create a span for the `call_correlated` operation.
///
/// The `correlation_id` and `result` fields are recorded during the call
/// using [`Span::record`].
#[expect(
    clippy::cognitive_complexity,
    reason = "complexity from dynamic_span! macro expansion"
)]
pub(crate) fn call_correlated_span(config: &TracingConfig) -> Span {
    dynamic_span!(
        config.call_level,
        "client.call_correlated",
        correlation_id = tracing::field::Empty,
        result = tracing::field::Empty
    )
}

/// Create a span for the `call_streaming` operation.
#[expect(
    clippy::cognitive_complexity,
    reason = "complexity from dynamic_span! macro expansion"
)]
pub(crate) fn streaming_span(config: &TracingConfig, correlation_id: u64) -> Span {
    dynamic_span!(
        config.streaming_level,
        "client.call_streaming",
        correlation_id = correlation_id,
        frame.bytes = tracing::field::Empty
    )
}

/// Create a span for the `close` operation.
#[expect(
    clippy::cognitive_complexity,
    reason = "complexity from dynamic_span! macro expansion"
)]
pub(crate) fn close_span(config: &TracingConfig) -> Span {
    dynamic_span!(config.close_level, "client.close")
}

/// Record elapsed time if timing was enabled for this operation.
///
/// The `start` parameter is `None` when timing is disabled and
/// `Some(instant)` when enabled. When `Some`, an event is emitted with
/// the `elapsed_us` field at `DEBUG` level.
pub(crate) fn emit_timing_event(start: Option<Instant>) {
    if let Some(start) = start {
        let elapsed_us = start.elapsed().as_micros();
        tracing::debug!(elapsed_us = elapsed_us, "operation.timing");
    }
}
