//! Pin `ServerShutdown`'s and `ServerError`'s auto-trait contracts.
//!
//! The module-scope `assert_impl_all!` calls in the library enforce these
//! contracts on every normal build. This case re-checks them from outside the
//! crate, so the guarantee is stated at the public API boundary that a
//! downstream test-support handle will rely on: a running control crosses
//! tasks, is shared by reference, and exposes a `Send` drain future.
//!
//! A compile-fail counterpart cannot exist, and that absence is structural
//! rather than an oversight. `ServerShutdown::new` is `pub(in crate::server)`
//! and `ServerError` is a non-generic enum whose payloads are concrete, so no
//! downstream crate can hand either type a `!Sync` payload. A regression
//! therefore surfaces here, where these bounds stop compiling, rather than as
//! an externally constructible failing case.
//!
//! The bounds are written with plain generic helpers rather than
//! `static_assertions`, so this fixture depends only on `wireframe` itself.

use std::future::Future;

use wireframe::server::{ServerError, ServerShutdown};

/// Require `Send`, `Sync` and `Unpin` of a type, without needing a value.
fn assert_auto_traits<T: Send + Sync + Unpin>() {}

/// Require `Send` and `Sync` of a type, without needing a value.
fn assert_send_sync<T: Send + Sync>() {}

/// The drain future borrows the control, and a multi-threaded spawn needs it
/// to be `Send`.
fn drained_is_send(
    control: &ServerShutdown,
) -> impl Future<Output = Result<(), ServerError>> + Send + '_ {
    control.drained()
}

fn main() {
    assert_auto_traits::<ServerShutdown>();
    assert_send_sync::<ServerError>();

    // Referencing the helper type-checks its body without a live server.
    let _ = drained_is_send;
}
