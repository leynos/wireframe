//! Tests for codec recovery policies.

use std::{io, time::Duration};

use super::*;
use crate::codec::{
    CodecError,
    error::{EofError, FramingError},
};

#[test]
fn recovery_policy_default_is_drop() {
    assert_eq!(
        RecoveryPolicy::default(),
        RecoveryPolicy::Drop,
        "RecoveryPolicy::default() should equal RecoveryPolicy::Drop"
    );
}

#[test]
fn context_builder_sets_fields() {
    let ctx = CodecErrorContext::new()
        .with_connection_id(42)
        .with_correlation_id(123)
        .with_codec_state("seq=5");

    assert_eq!(
        ctx.connection_id,
        Some(42),
        "ctx.connection_id should equal Some(42)"
    );
    assert_eq!(
        ctx.correlation_id,
        Some(123),
        "ctx.correlation_id should equal Some(123)"
    );
    assert_eq!(
        ctx.codec_state,
        Some("seq=5".to_owned()),
        "ctx.codec_state should equal Some(\"seq=5\".to_string())"
    );
}

#[test]
fn context_with_peer_address() {
    let addr: std::net::SocketAddr = "127.0.0.1:8080".parse().expect("valid test address");
    let ctx = CodecErrorContext::new().with_peer_address(addr);
    assert_eq!(
        ctx.peer_address,
        Some(addr),
        "ctx.peer_address should equal Some(addr)"
    );
}

#[test]
fn default_recovery_policy_delegates_to_error() {
    let default_hook = DefaultRecoveryPolicy;
    let default_ctx = CodecErrorContext::new();

    // Check various error types
    let err = CodecError::Framing(FramingError::OversizedFrame { size: 100, max: 50 });
    assert_eq!(
        default_hook.recovery_policy(&err, &default_ctx),
        RecoveryPolicy::Drop,
        "default_hook.recovery_policy(&err, &default_ctx) should equal RecoveryPolicy::Drop"
    );

    let io_err = CodecError::Io(io::Error::other("test"));
    assert_eq!(
        default_hook.recovery_policy(&io_err, &default_ctx),
        RecoveryPolicy::Disconnect,
        "default_hook.recovery_policy(&io_err, &default_ctx) should equal \
         RecoveryPolicy::Disconnect"
    );

    let eof_err = CodecError::Eof(EofError::CleanClose);
    assert_eq!(
        default_hook.recovery_policy(&eof_err, &default_ctx),
        RecoveryPolicy::Disconnect,
        "default_hook.recovery_policy(&eof_err, &default_ctx) should equal \
         RecoveryPolicy::Disconnect"
    );
}

#[test]
fn default_quarantine_duration_is_30_seconds() {
    let default_hook = DefaultRecoveryPolicy;
    let default_ctx = CodecErrorContext::new();
    let err = CodecError::Io(io::Error::other("test"));

    assert_eq!(
        default_hook.quarantine_duration(&err, &default_ctx),
        Duration::from_secs(30),
        "default_hook.quarantine_duration(&err, &default_ctx) should equal Duration::from_secs(30)"
    );
}

#[test]
fn recovery_config_builder() {
    let config = RecoveryConfig::default()
        .max_consecutive_drops(5)
        .quarantine_duration(Duration::from_mins(1))
        .log_dropped_frames(false);

    assert_eq!(
        config.max_consecutive_drops, 5,
        "config.max_consecutive_drops should equal 5"
    );
    assert_eq!(
        config.quarantine_duration,
        Duration::from_mins(1),
        "config.quarantine_duration should equal Duration::from_mins(1)"
    );
    assert!(
        !config.log_dropped_frames,
        "assertion failed: !config.log_dropped_frames"
    );
}

#[test]
fn recovery_config_defaults() {
    let config = RecoveryConfig::default();

    assert_eq!(
        config.max_consecutive_drops, 10,
        "config.max_consecutive_drops should equal 10"
    );
    assert_eq!(
        config.quarantine_duration,
        Duration::from_secs(30),
        "config.quarantine_duration should equal Duration::from_secs(30)"
    );
    assert!(
        config.log_dropped_frames,
        "assertion failed: config.log_dropped_frames"
    );
}
