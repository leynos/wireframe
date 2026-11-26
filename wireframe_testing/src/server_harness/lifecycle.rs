//! Startup helpers for the server lifecycle harness.
//!
//! Every entry point performs the same five steps: reserve a listener, read
//! its address, install a harness-owned readiness sender, start the server
//! with [`WireframeServer::spawn`], and await readiness under the configured
//! bound. A failure at any step maps onto the typed [`TestError`] variant for
//! that stage, and a failed readiness wait stops the server before returning.
//!
//! The helpers take over the readiness signal. `ready_signal` cannot be
//! enforced, and the server exposes no accessor that would reveal a
//! caller-set sender, so a sender installed before the call never fires. Do
//! not configure `ready_signal` on a server you pass here.

use std::{
    net::{SocketAddr, TcpListener as StdTcpListener},
    time::Duration,
};

use tokio::{sync::oneshot, time::timeout};
use wireframe::{
    app::{Envelope, Packet},
    codec::FrameCodec,
    frame::FrameMetadata,
    message::{DecodeWith, EncodeWith},
    preamble::Preamble,
    serializer::Serializer,
    server::{AppFactory, Bound, ServerShutdown, Unbound, WireframeServer},
};

use super::{handle::RunningWireframeServer, options::ServerHarnessOptions};
use crate::{TestError, TestResult, integration_helpers::unused_listener};

/// Emit one public spawn entry point with the server's `spawn` bounds.
///
/// Both entry points share the same seven generic predicates, which the
/// server's `spawn` impl requires verbatim. Writing them once here keeps the
/// public signatures identical while avoiding a second copy of the clause.
macro_rules! spawn_entry_point {
    (
        $(#[$docs:meta])*
        fn $name:ident($server:ident $(, $listener:ident : $listener_ty:ty)?) $body:block
    ) => {
        $(#[$docs])*
        pub async fn $name<F, T, Ser, Ctx, E, Codec>(
            $server: WireframeServer<F, T, Unbound, Ser, Ctx, E, Codec>,
            $($listener: $listener_ty,)?
        ) -> TestResult<RunningWireframeServer>
        where
            F: AppFactory<Ser, Ctx, E, Codec>,
            T: Preamble,
            Ser: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync + 'static,
            Ctx: Send + 'static,
            E: Packet + 'static,
            Codec: FrameCodec,
            Envelope: DecodeWith<Ser> + EncodeWith<Ser>,
        {
            $body
        }
    };
}

spawn_entry_point! {
    /// Start an unbound server on a reserved loopback listener.
    ///
    /// The server is configured entirely by the caller; this helper only
    /// reserves the listener, binds it, and waits for readiness.
    ///
    /// # Errors
    ///
    /// Returns [`TestError::Io`] if the listener cannot be reserved or its
    /// address read, [`TestError::Server`] if binding or startup fails, and
    /// [`TestError::Timeout`] if readiness is not observed within the
    /// configured bound.
    fn spawn_wireframe_server(server) {
        spawn_wireframe_server_on(server, unused_listener()?).await
    }
}

spawn_entry_point! {
    /// Start an unbound server on a caller-supplied listener.
    ///
    /// Use this when the test must know the address before the server starts,
    /// for example to pass it to a peer that connects during setup.
    ///
    /// # Errors
    ///
    /// Returns [`TestError::Io`] if the listener's address cannot be read,
    /// [`TestError::Server`] if binding or startup fails, and
    /// [`TestError::Timeout`] if readiness is not observed within the
    /// configured bound.
    fn spawn_wireframe_server_on(server, listener: StdTcpListener) {
        spawn_wireframe_server_with_options(
            server,
            Some(listener),
            ServerHarnessOptions::default(),
        )
        .await
    }
}

/// Start an unbound server with an explicit listener and timeout policy.
///
/// This is the shared implementation behind the other two entry points, and
/// the form the pair helper uses so it can pass unbounded waits.
///
/// # Errors
///
/// Returns [`TestError::Io`] if the listener cannot be reserved or its address
/// read, [`TestError::Server`] if binding or startup fails, and
/// [`TestError::Timeout`] if readiness is not observed within the configured
/// bound. When a failed readiness wait is followed by a failed cleanup, the
/// combined failure is reported as [`TestError::Msg`] naming both causes.
pub async fn spawn_wireframe_server_with_options<F, T, Ser, Ctx, E, Codec>(
    server: WireframeServer<F, T, Unbound, Ser, Ctx, E, Codec>,
    listener: Option<StdTcpListener>,
    options: ServerHarnessOptions,
) -> TestResult<RunningWireframeServer>
where
    F: AppFactory<Ser, Ctx, E, Codec>,
    T: Preamble,
    Ser: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync + 'static,
    Ctx: Send + 'static,
    E: Packet + 'static,
    Codec: FrameCodec,
    Envelope: DecodeWith<Ser> + EncodeWith<Ser>,
{
    let reserved = match listener {
        Some(supplied) => supplied,
        None => unused_listener()?,
    };
    let (bound, addr, ready_rx) = bind_with_readiness(server, reserved)?;
    start_bound(bound, addr, ready_rx, options).await
}

/// A bound server, the address it was bound to, and its readiness receiver.
type BoundWithReadiness<F, T, Ser, Ctx, E, Codec> = (
    WireframeServer<F, T, Bound, Ser, Ctx, E, Codec>,
    SocketAddr,
    oneshot::Receiver<()>,
);

/// Reserve the address, install the readiness sender, and bind the server.
///
/// This is the first half of the startup sequence, exposed to the pair helper
/// so its binding failures keep their typed variants. The returned receiver is
/// resolved by the supervisor once its accept loops are running.
pub(crate) fn bind_with_readiness<F, T, Ser, Ctx, E, Codec>(
    server: WireframeServer<F, T, Unbound, Ser, Ctx, E, Codec>,
    listener: StdTcpListener,
) -> TestResult<BoundWithReadiness<F, T, Ser, Ctx, E, Codec>>
where
    F: AppFactory<Ser, Ctx, E, Codec>,
    T: Preamble,
    Ser: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync + 'static,
    Ctx: Send + 'static,
    E: Packet + 'static,
    Codec: FrameCodec,
    Envelope: DecodeWith<Ser> + EncodeWith<Ser>,
{
    let addr = listener.local_addr()?;
    let (ready_tx, ready_rx) = oneshot::channel();
    let bound = server
        .ready_signal(ready_tx)
        .bind_existing_listener(listener)?;
    Ok((bound, addr, ready_rx))
}

/// Start a bound server and await readiness, cleaning up on failure.
///
/// This is the second half of the startup sequence. Its failures are the ones
/// the pair helper re-wraps as [`TestError::Msg`]; binding failures stay typed
/// because they are produced by [`bind_with_readiness`].
pub(crate) async fn start_bound<F, T, Ser, Ctx, E, Codec>(
    server: WireframeServer<F, T, Bound, Ser, Ctx, E, Codec>,
    addr: SocketAddr,
    ready_rx: oneshot::Receiver<()>,
    options: ServerHarnessOptions,
) -> TestResult<RunningWireframeServer>
where
    F: AppFactory<Ser, Ctx, E, Codec>,
    T: Preamble,
    Ser: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync + 'static,
    Ctx: Send + 'static,
    E: Packet + 'static,
    Codec: FrameCodec,
    Envelope: DecodeWith<Ser> + EncodeWith<Ser>,
{
    let control = server.spawn().await?;
    await_readiness(ready_rx, &options, &control).await?;
    Ok(RunningWireframeServer::new(addr, control, options))
}

/// Wait for the readiness signal, stopping the server if the wait fails.
///
/// A live server never drains unasked, so a failed wait must stop it before
/// returning or the harness would leak a running supervisor.
async fn await_readiness(
    ready_rx: oneshot::Receiver<()>,
    options: &ServerHarnessOptions,
    control: &ServerShutdown,
) -> TestResult<()> {
    let waited = match options.readiness_timeout() {
        Some(bound) => timeout(bound, ready_rx).await,
        None => Ok(ready_rx.await),
    };
    match waited {
        Ok(Ok(())) => Ok(()),
        Ok(Err(_dropped)) => {
            let cleanup = clean_up(control, options.shutdown_timeout()).await;
            match cleanup {
                Ok(()) => Err(TestError::Msg(
                    "server exited before signalling ready".to_owned(),
                )),
                // The plan keeps this defensive arm's cleanup failure typed:
                // callers distinguish a server error from a readiness timeout.
                Err(error) => Err(error),
            }
        }
        Err(elapsed) => {
            let cleanup = clean_up(control, options.shutdown_timeout()).await;
            match cleanup {
                Ok(()) => Err(TestError::Timeout(elapsed)),
                Err(error) => Err(TestError::Msg(format!(
                    "readiness timed out; cleanup also failed: {error}"
                ))),
            }
        }
    }
}

/// Stop a server whose startup failed and wait for its drain.
async fn clean_up(control: &ServerShutdown, configured: Option<Duration>) -> TestResult<()> {
    control.stop();
    match configured {
        Some(limit) => timeout(limit, control.drained()).await??,
        None => control.drained().await?,
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    //! Readiness-arm tests that cannot be reached through the public API.

    use std::time::Duration;

    use tokio::sync::oneshot;
    use wireframe::server::WireframeServer;

    use super::{ServerHarnessOptions, await_readiness};
    use crate::{
        TestError,
        TestResult,
        integration_helpers::TestApp,
        wait_for_listener_release,
        wait_for_server_readiness,
    };

    /// Spawn a ready server plus the control used to exercise the arms.
    async fn spawn_ready_control()
    -> TestResult<(std::net::SocketAddr, wireframe::server::ServerShutdown)> {
        let listener = crate::unused_listener()?;
        let server = WireframeServer::new(|| -> TestApp { TestApp::default() })
            .workers(1)
            .bind_existing_listener(listener)?;
        let addr = server.local_addr().ok_or("server local address missing")?;
        let (ready_tx, ready_rx) = oneshot::channel();
        let control = server.ready_signal(ready_tx).spawn().await?;
        wait_for_server_readiness(ready_rx).await?;
        Ok((addr, control))
    }

    #[tokio::test]
    async fn elapsed_readiness_stops_the_server() -> TestResult {
        let (addr, control) = spawn_ready_control().await?;
        let (held_tx, held_rx) = oneshot::channel::<()>();
        let options =
            ServerHarnessOptions::default().with_readiness_timeout(Some(Duration::from_millis(5)));

        let error = await_readiness(held_rx, &options, &control).await;
        drop(held_tx);
        if !matches!(error, Err(TestError::Timeout(_))) {
            return Err(
                format!("an unsent readiness sender must report the timeout: {error:?}").into(),
            );
        }
        wait_for_listener_release(addr).await
    }

    #[tokio::test]
    async fn dropped_readiness_sender_stops_the_server() -> TestResult {
        let (addr, control) = spawn_ready_control().await?;
        let (dropped_tx, dropped_rx) = oneshot::channel::<()>();
        drop(dropped_tx);
        let options = ServerHarnessOptions::default();

        let error = await_readiness(dropped_rx, &options, &control).await;
        match error {
            Err(TestError::Msg(message)) if message == "server exited before signalling ready" => {}
            other => {
                return Err(format!("expected the readiness message, got {other:?}").into());
            }
        }
        wait_for_listener_release(addr).await
    }
}
