//! Verifies public server-runtime APIs remain callable in const contexts.
//!
//! The lint-census layer made the push-queue builder setters, `ConnectionId`
//! accessors, `ConnectionInfo::peer_addr`, and several error and codec
//! accessors `const fn`. Const callability is a compile-time promise, so a
//! fixture that compiles is the only check that can catch a regression.
//!
//! Values that no `const fn` can construct, such as `io::Error` or a live
//! connection, arrive as parameters: the point under test is the callee, not
//! the argument.

use std::{net::SocketAddr, time::Duration};

use wireframe::{
    codec::{CodecError, EofError, LengthDelimitedFrameCodec},
    error::WireframeError,
    extractor::ConnectionInfo,
    push::PushQueuesBuilder,
    session::ConnectionId,
};

/// Every builder setter chains, and each stays `const`, inside a `const fn`.
/// The frame parameter stays generic: `FrameLike` is only crate-visible, but
/// its blanket implementation means `F: Send + 'static` is all a caller needs.
const fn configure_builder<F: Send + 'static>(
    builder: PushQueuesBuilder<F>,
) -> PushQueuesBuilder<F> {
    builder
        .high_capacity(8)
        .low_capacity(4)
        .rate(Some(64))
        .dlq_log_every_n(2)
        .dlq_log_interval(Duration::from_secs(5))
}

/// Removing the rate limiter and restoring one both stay `const`.
const fn recalibrate_builder<F: Send + 'static>(
    builder: PushQueuesBuilder<F>,
) -> PushQueuesBuilder<F> {
    builder.unlimited().rate(Some(16))
}

const CONNECTION_ID: ConnectionId = ConnectionId::new(42);
const CONNECTION_ID_VALUE: u64 = CONNECTION_ID.as_u64();

/// The codec accessor is const-callable. A codec value is not `const`
/// constructible, so it arrives as a parameter.
const fn frame_limit(codec: &LengthDelimitedFrameCodec) -> usize {
    codec.max_frame_length()
}

/// `ConnectionInfo::peer_addr` is const; a live value arrives as a parameter.
const fn peer_addr_of(info: &ConnectionInfo) -> Option<SocketAddr> { info.peer_addr() }

/// `WireframeError::from_codec` is const, and the codec error it wraps is
/// const-constructible, so the whole value is built in a `const` item.
const CLEAN_CLOSE_CODEC: CodecError = CodecError::Eof(EofError::CleanClose);
const CLEAN_CLOSE_ERROR: WireframeError = WireframeError::from_codec(CLEAN_CLOSE_CODEC);

/// A `WireframeError` has a non-const destructor, so it can be neither dropped
/// nor consumed inside a `const fn`. Borrowing it keeps the value alive and
/// still exercises the const `is_clean_close` receiver.
const fn is_clean_close(error: &WireframeError) -> bool { error.is_clean_close() }

/// Returning the error moves it out instead of dropping it, which is the only
/// shape that lets a `const fn` wrap a non-const-constructible payload.
const fn wrap_io(error: std::io::Error) -> WireframeError {
    WireframeError::from_io(error)
}

fn main() {
    let _ = (
        configure_builder::<Vec<u8>>,
        recalibrate_builder::<Vec<u8>>,
        CONNECTION_ID_VALUE,
        frame_limit,
        peer_addr_of,
        CLEAN_CLOSE_ERROR.is_clean_close(),
        is_clean_close(&CLEAN_CLOSE_ERROR),
        wrap_io,
    );
}
