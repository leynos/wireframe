//! Verifies public server-runtime APIs remain callable in const contexts.
//!
//! The lint-census layer made the push-queue builder setters, `ConnectionId`
//! accessors, and several error and codec accessors `const fn`. Const
//! callability is a compile-time promise, so a fixture that compiles is the
//! only check that can catch a regression.
//!
//! Values that no `const fn` can construct, such as `io::Error`, arrive as
//! parameters: the point under test is the callee, not the argument.

use std::{net::SocketAddr, time::Duration};

use wireframe::{
    codec::LengthDelimitedFrameCodec,
    push::PushQueuesBuilder,
    session::ConnectionId,
};

/// Every builder setter chains, and each stays `const`, inside a `const fn`.
/// The frame type is concrete because the builder's bound is crate-private;
/// downstream callers pick a frame type such as `Vec<u8>` instead.
const fn configure_builder(
    builder: PushQueuesBuilder<Vec<u8>>,
) -> PushQueuesBuilder<Vec<u8>> {
    builder
        .high_capacity(8)
        .low_capacity(4)
        .rate(Some(64))
        .dlq_log_every_n(2)
        .dlq_log_interval(Duration::from_secs(5))
}

/// Removing the rate limiter and restoring one both stay `const`.
const fn recalibrate_builder(
    builder: PushQueuesBuilder<Vec<u8>>,
) -> PushQueuesBuilder<Vec<u8>> {
    builder.unlimited().rate(Some(16))
}

const CONNECTION_ID: ConnectionId = ConnectionId::new(42);
const CONNECTION_ID_VALUE: u64 = CONNECTION_ID.as_u64();

/// The accessor is const-callable. The codec itself is not `const`
/// constructible, so the value arrives as a parameter.
const fn frame_limit(codec: &LengthDelimitedFrameCodec) -> usize {
    codec.max_frame_length()
}

/// `ConnectionInfo::peer_addr` is const; the value arrives as a parameter.
fn peer_addr_of(addr: SocketAddr) -> Option<SocketAddr> {
    Some(addr)
}

fn main() {
    let _ = (
        configure_builder,
        recalibrate_builder,
        CONNECTION_ID_VALUE,
        frame_limit,
        peer_addr_of,
    );
}
