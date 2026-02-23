//! Assertion macros shared by test helpers and integration tests.

/// Await a push future and return a contextualized error on failure.
#[macro_export]
macro_rules! push_expect {
    ($fut:expr) => {{
        $fut.await.map_err(|err| {
            ::std::io::Error::other(::std::format!(
                "push failed at {}:{}: {err}",
                file!(),
                line!()
            ))
        })
    }};
    ($fut:expr, $msg:expr) => {{
        $fut.await.map_err(|err| {
            ::std::io::Error::other(::std::format!(
                "{msg} at {}:{}: {err}",
                file!(),
                line!(),
                msg = $msg
            ))
        })
    }};
}

/// Await a receive future and return a contextualized error on failure.
#[macro_export]
macro_rules! recv_expect {
    ($fut:expr) => {{
        $fut.await.ok_or_else(|| {
            ::std::io::Error::other(::std::format!(
                "recv failed at {}:{}: channel closed",
                file!(),
                line!()
            ))
        })
    }};
    ($fut:expr, $msg:expr) => {{
        $fut.await.ok_or_else(|| {
            ::std::io::Error::other(::std::format!(
                "{msg} at {}:{}: channel closed",
                file!(),
                line!(),
                msg = $msg
            ))
        })
    }};
}

pub use crate::{push_expect, recv_expect};
