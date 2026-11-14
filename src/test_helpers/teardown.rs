//! Shared unit-state connection teardown fixtures.
//!
//! Unit tests and integration tests both need a teardown callback that counts
//! how many connections finalized. One definition here keeps those copies from
//! drifting apart and makes "exactly once" assertions comparable across test
//! crates.

use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

/// Boxed future returned by unit-state lifecycle callbacks.
pub type UnitFuture = Pin<Box<dyn Future<Output = ()> + Send>>;

/// Build a teardown callback for unit connection state that counts
/// invocations.
///
/// Each invocation increments the shared counter by one, so a test can assert
/// that teardown ran exactly once per connection.
///
/// ```rust,no_run
/// use std::sync::{
///     Arc,
///     atomic::{AtomicUsize, Ordering},
/// };
///
/// use wireframe::test_helpers::counting_teardown;
///
/// let calls = Arc::new(AtomicUsize::new(0));
/// let teardown = counting_teardown(Arc::clone(&calls));
/// futures::executor::block_on(teardown(()));
/// assert_eq!(calls.load(Ordering::SeqCst), 1);
/// ```
pub fn counting_teardown(
    calls: Arc<AtomicUsize>,
) -> impl Fn(()) -> UnitFuture + Send + Sync + 'static {
    move |()| {
        let counter = Arc::clone(&calls);
        Box::pin(async move {
            counter.fetch_add(1, Ordering::SeqCst);
        })
    }
}
