//! Abort-and-join ownership for server tasks spawned by async test harnesses.
//!
//! A test helper that spawns a server before building a client has three ways
//! to leave: it can succeed, it can return an error before the client ever
//! connects, or its body can panic. Awaiting a bare [`JoinHandle`] covers only
//! the first. On the other two the server may be parked in `accept` with no
//! client to serve, so a natural join never finishes: the test hangs or leaks
//! the task, and the server's own failure is never observed.
//!
//! [`ServerTaskGuard`] owns the handle across every await and reaps the task on
//! all three paths. [`ServerTaskGuard::run_and_join`] waits for a server whose
//! loop ends with the client, and [`ServerTaskGuard::run_and_abort`] stops a
//! long-lived one. Both abort and await the task before propagating a body
//! error or resuming a body panic, so the body outcome wins while the server is
//! still stopped and observed.

use std::{
    future::Future,
    panic::{AssertUnwindSafe, resume_unwind},
};

use futures::FutureExt as _;
use tokio::task::{JoinError, JoinHandle};

/// Panic payload recovered by [`futures::FutureExt::catch_unwind`].
///
/// Naming it keeps the runners readable and makes it obvious that they forward
/// the payload untouched rather than inspecting it.
type PanicPayload = Box<dyn std::any::Any + Send>;

/// Owns a spawned server task and reaps it on every exit path.
///
/// The guard is single-use: each consuming method awaits the handle exactly
/// once. Dropping a guard that was never reaped aborts the task, which is the
/// fallback for a cleanup future that is itself cancelled — cancellation cannot
/// await, so the task stops but its outcome is discarded.
#[must_use = "a dropped guard aborts the server without observing its outcome"]
pub struct ServerTaskGuard<T> {
    /// The owned task handle; it is awaited at most once.
    handle: JoinHandle<T>,
    /// Whether dropping without reaping should abort the task.
    ///
    /// Cleared as soon as the handle has been awaited, because polling a
    /// completed [`JoinHandle`] again panics.
    should_abort_on_drop: bool,
}

impl<T> ServerTaskGuard<T> {
    /// Take ownership of a spawned server task.
    pub const fn new(handle: JoinHandle<T>) -> Self {
        Self {
            handle,
            should_abort_on_drop: true,
        }
    }

    /// Wait for the server to finish and return its value.
    ///
    /// Use this for a server whose loop ends once the test client disconnects,
    /// such as an echo server driven to a natural close. Awaiting while the
    /// client is still connected blocks, so release the client first.
    ///
    /// # Errors
    ///
    /// Returns the task's [`JoinError`] when it panicked or was aborted.
    pub async fn join(mut self) -> Result<T, JoinError> {
        let outcome = (&mut self.handle).await;
        self.should_abort_on_drop = false;
        outcome
    }

    /// Abort the server, wait for it to stop, and report what it produced.
    ///
    /// A task that had already finished yields its value; a task stopped by
    /// this abort yields `None`. Both outcomes are clean, so only a panic is
    /// reported as an error.
    ///
    /// # Errors
    ///
    /// Returns the task's [`JoinError`] when it panicked before the abort took
    /// effect.
    pub async fn abort_and_join(mut self) -> Result<Option<T>, JoinError> {
        self.handle.abort();
        let outcome = (&mut self.handle).await;
        self.should_abort_on_drop = false;
        match outcome {
            Ok(value) => Ok(Some(value)),
            Err(error) if error.is_cancelled() => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Run `body`, then wait for the server and return both outcomes.
    ///
    /// Connection setup, the test's assertions, and the client's last use all
    /// belong inside `body`: that is what puts their errors and panics under
    /// the guard. The body must release the client before it returns, or the
    /// natural join has no disconnect to end the serve loop.
    ///
    /// On success the server is joined and its outcome returned, so a
    /// server-side failure is never reported as a clean pass. A body error or
    /// panic takes precedence over the cleanup outcome: the task is aborted and
    /// awaited first, then the original error is returned or the original panic
    /// payload is resumed.
    ///
    /// # Errors
    ///
    /// Returns `body`'s own error unchanged, or the [`JoinError`] produced when
    /// the server panicked after a successful body.
    ///
    /// # Panics
    ///
    /// Resumes the payload when `body` panics, after the server task has been
    /// aborted and awaited.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # use std::error::Error;
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    /// use wireframe::test_helpers::ServerTaskGuard;
    ///
    /// let server = tokio::spawn(async { 7_u32 });
    /// let (body_value, server_value) = ServerTaskGuard::new(server)
    ///     .run_and_join(async { Ok::<_, Box<dyn Error + Send + Sync>>(5_u32) })
    ///     .await?;
    /// assert_eq!((body_value, server_value), (5, 7));
    /// # Ok(())
    /// # }
    /// ```
    pub async fn run_and_join<R, E, Fut>(self, body: Fut) -> Result<(R, T), E>
    where
        Fut: Future<Output = Result<R, E>>,
        E: From<JoinError>,
    {
        self.run_guarded(body, Self::join).await
    }

    /// Run `body`, then abort the server and return both outcomes.
    ///
    /// Use this for a server that would otherwise outlive the test, such as a
    /// sample server looping on requests the body no longer sends. As with
    /// [`Self::run_and_join`], the body owns the client work and its errors and
    /// panics take precedence over the cleanup outcome.
    ///
    /// # Errors
    ///
    /// Returns `body`'s own error unchanged, or the [`JoinError`] produced when
    /// the server panicked before the abort took effect.
    ///
    /// # Panics
    ///
    /// Resumes the payload when `body` panics, after the server task has been
    /// aborted and awaited.
    pub async fn run_and_abort<R, E, Fut>(self, body: Fut) -> Result<(R, Option<T>), E>
    where
        Fut: Future<Output = Result<R, E>>,
        E: From<JoinError>,
    {
        self.run_guarded(body, Self::abort_and_join).await
    }

    /// Drive `body` under the guard, reaping the server with `reap` on success.
    ///
    /// The two public runners differ only in how they treat a successful body,
    /// so that choice is the parameter and everything else is shared. `reap`
    /// owns the guard only on the success path; a returned error or a resumed
    /// panic stops and awaits the server first, so no path leaves it running.
    async fn run_guarded<R, E, Fut, S, F, G>(self, body: Fut, reap: F) -> Result<(R, S), E>
    where
        Fut: Future<Output = Result<R, E>>,
        E: From<JoinError>,
        F: FnOnce(Self) -> G,
        G: Future<Output = Result<S, JoinError>>,
    {
        match AssertUnwindSafe(body).catch_unwind().await {
            Ok(Ok(value)) => {
                let server = reap(self).await.map_err(E::from)?;
                Ok((value, server))
            }
            Ok(Err(error)) => {
                let _cleanup = self.abort_and_join().await;
                Err(error)
            }
            // `stop_and_resume` diverges; the `Err` arm only satisfies the
            // return type.
            Err(payload) => Err(stop_and_resume(self, payload).await),
        }
    }
}

/// Abort and await the server, then resume the body's panic.
///
/// Cleanup happens before the panic continues, so a failing body never leaves
/// the server running. The return type names the body's error only so a caller
/// can place the call in an `Err` arm; it always diverges.
async fn stop_and_resume<T, E>(guard: ServerTaskGuard<T>, payload: PanicPayload) -> E {
    let _cleanup = guard.abort_and_join().await;
    resume_unwind(payload)
}

impl<T> std::fmt::Debug for ServerTaskGuard<T> {
    /// Report the guard's armed state without requiring `T: Debug`.
    ///
    /// A fixture that derives `Debug` holds the guard, and the task's value is
    /// never inspectable before it is awaited, so showing it would say nothing.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerTaskGuard")
            .field("should_abort_on_drop", &self.should_abort_on_drop)
            .finish_non_exhaustive()
    }
}

impl<T> Drop for ServerTaskGuard<T> {
    /// Abort the task when the guard is dropped without reaping it.
    ///
    /// This is the cancellation fallback for a guard that never reaches a
    /// consuming method, such as one held by a future that is itself dropped.
    /// It needs no runtime, because `abort` only marks the task; the outcome is
    /// necessarily discarded.
    fn drop(&mut self) {
        if self.should_abort_on_drop {
            self.handle.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    //! Behavioural tests for the guard's success, error, panic, and drop paths.

    use tokio::sync::oneshot;

    use super::*;

    /// Error type used by the guarded bodies in these tests.
    type BodyError = Box<dyn std::error::Error + Send + Sync>;

    /// Spawn a server that never finishes and report when its task is dropped.
    ///
    /// Holding the sender inside the task keeps the channel open for exactly as
    /// long as the task runs, so the receiver closes when the guard reaps it.
    fn spawn_pending_server() -> (ServerTaskGuard<()>, oneshot::Receiver<()>) {
        let (sender, receiver) = oneshot::channel();
        let handle = tokio::spawn(async move {
            let _keep_alive = sender;
            std::future::pending::<()>().await;
        });
        (ServerTaskGuard::new(handle), receiver)
    }

    /// Whether the task holding the channel's sender has been dropped.
    fn is_reaped(receiver: &mut oneshot::Receiver<()>) -> bool {
        matches!(
            receiver.try_recv(),
            Err(oneshot::error::TryRecvError::Closed)
        )
    }

    /// Yield until an abort has taken effect and its task has been dropped.
    ///
    /// `abort` only marks the task, so the scheduler needs a turn before the
    /// task's future, and the sender it holds, are dropped.
    async fn wait_until_reaped(receiver: &mut oneshot::Receiver<()>) {
        for _ in 0..64 {
            if is_reaped(receiver) {
                return;
            }
            tokio::task::yield_now().await;
        }
    }

    /// Assert that the guarded server task has finished and released its state.
    fn assert_reaped(receiver: &mut oneshot::Receiver<()>, context: &str) {
        let outcome = receiver.try_recv();
        assert!(
            matches!(outcome, Err(oneshot::error::TryRecvError::Closed)),
            "{context}: the server task must be dropped once the guard reaps it, got {outcome:?}"
        );
    }

    #[tokio::test]
    async fn run_and_join_returns_the_body_and_server_values() {
        let handle = tokio::spawn(async { 7_u32 });

        let (body_value, server_value) = ServerTaskGuard::new(handle)
            .run_and_join(async { Ok::<_, BodyError>(5_u32) })
            .await
            .expect("join a completed server");

        assert_eq!(body_value, 5, "the body value must be returned");
        assert_eq!(server_value, 7, "the server value must be returned");
    }

    #[tokio::test]
    async fn run_and_join_returns_a_body_error_and_reaps_the_server() {
        let (guard, mut receiver) = spawn_pending_server();

        let outcome = guard
            .run_and_join(async { Err::<(), _>(std::io::Error::other("body failed")) })
            .await;

        let error = outcome.expect_err("a body error must be returned");
        assert_eq!(
            error.to_string(),
            "body failed",
            "the body error must survive"
        );
        assert_reaped(&mut receiver, "body error");
    }

    #[tokio::test]
    async fn run_and_join_resumes_a_body_panic_after_reaping_the_server() {
        let (guard, mut receiver) = spawn_pending_server();

        let outcome = AssertUnwindSafe(
            guard.run_and_join::<(), BodyError, _>(async { panic!("body panicked") }),
        )
        .catch_unwind()
        .await;

        let payload = outcome.expect_err("a body panic must be resumed");
        let message = payload.downcast_ref::<&str>().copied();
        assert_eq!(
            message,
            Some("body panicked"),
            "the original panic payload must be resumed"
        );
        assert_reaped(&mut receiver, "body panic");
    }

    #[tokio::test]
    async fn run_and_join_surfaces_a_server_panic_after_body_success() {
        // The panic is deliberate and the runtime reports it to stderr.
        let handle = tokio::spawn(async { panic!("server panicked") });

        let outcome = ServerTaskGuard::new(handle)
            .run_and_join(async { Ok::<_, BodyError>(()) })
            .await;

        assert!(
            outcome.is_err(),
            "a server panic must fail the runner instead of reporting success"
        );
    }

    #[tokio::test]
    async fn run_and_abort_treats_cancellation_as_clean() {
        let (guard, mut receiver) = spawn_pending_server();

        let ((), server_value) = guard
            .run_and_abort(async { Ok::<_, BodyError>(()) })
            .await
            .expect("aborting a pending server is a clean outcome");

        assert!(server_value.is_none(), "an aborted server reports no value");
        assert_reaped(&mut receiver, "aborted server");
    }

    #[tokio::test]
    async fn run_and_abort_surfaces_a_server_panic() {
        // An abort can pre-empt a task that has not run yet, in which case the
        // panic never happens and the outcome is a clean cancellation. Wait for
        // the task to start so the panic is guaranteed to precede the abort.
        // The panic is deliberate and the runtime reports it to stderr.
        let (started, has_started) = oneshot::channel();
        let handle = tokio::spawn(async move {
            let _signalled = started.send(());
            panic!("server panicked");
        });
        has_started.await.expect("the server task must start");

        let outcome = ServerTaskGuard::new(handle)
            .run_and_abort(async { Ok::<_, BodyError>(()) })
            .await;

        assert!(
            outcome.is_err(),
            "a server panic must not be mistaken for a clean abort"
        );
    }

    #[tokio::test]
    async fn abort_on_drop_stops_an_unreaped_server() {
        let (guard, mut receiver) = spawn_pending_server();

        drop(guard);
        wait_until_reaped(&mut receiver).await;

        assert_reaped(&mut receiver, "dropped guard");
    }
}
