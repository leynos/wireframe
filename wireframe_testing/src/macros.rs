//! Assertion macros shared by test helpers and integration tests.

/// Await a push future and panic with contextual diagnostics on failure.
#[macro_export]
macro_rules! push_expect {
    ($fut:expr) => {{
        $fut.await
            .expect(concat!("push failed at ", file!(), ":", line!()))
    }};
    ($fut:expr, $msg:expr) => {{
        let m = ::std::format!("{msg} at {}:{}", file!(), line!(), msg = $msg);
        $fut.await.expect(&m)
    }};
}

/// Await a receive future and panic with contextual diagnostics on failure.
#[macro_export]
macro_rules! recv_expect {
    ($fut:expr) => {{
        $fut.await
            .expect(concat!("recv failed at ", file!(), ":", line!()))
    }};
    ($fut:expr, $msg:expr) => {{
        let m = ::std::format!("{msg} at {}:{}", file!(), line!(), msg = $msg);
        $fut.await.expect(&m)
    }};
}

pub use crate::{push_expect, recv_expect};
