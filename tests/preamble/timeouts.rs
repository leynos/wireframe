//! Timeout behaviour tests for preamble handling.

use std::io;

use bincode::error::DecodeError;
use futures::future::BoxFuture;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::oneshot,
    time::{Duration, Instant, timeout},
};
use wireframe::{
    app::WireframeApp,
    server::{DEFAULT_PREAMBLE_TIMEOUT, WireframeServer},
};
use wireframe_testing::{TestResult, factory};

use crate::support::{
    Holder,
    HotlinePreamble,
    channel_holder,
    failure_cb,
    notify_holder,
    recv_within,
    with_running_server,
};

const READ_TIMEOUT_MS: u64 = 500;

fn is_connection_closed(read_res: std::io::Result<usize>) -> TestResult<bool> {
    match read_res {
        Ok(0) => Ok(true),
        Ok(n) => {
            Err(io::Error::other(format!("expected connection to close, read {n} bytes")).into())
        }
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::ConnectionAborted | io::ErrorKind::ConnectionReset
            ) =>
        {
            Ok(true)
        }
        Err(e) => Err(e.into()),
    }
}

fn ensure_timed_out_decode_error(err: &DecodeError) -> io::Result<()> {
    if matches!(
        err,
        DecodeError::Io { inner, .. }
            if inner.kind() == io::ErrorKind::TimedOut
    ) {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "expected timed out error, got {err:?}"
        )))
    }
}

/// Test-side deadline that always outlives the server's default deadline.
///
/// Paused time auto-advances to the earliest pending timer, so the server's
/// deadline must be the first to elapse for these assertions to mean anything.
fn beyond_default_timeout() -> Duration { DEFAULT_PREAMBLE_TIMEOUT.saturating_mul(2) }

fn timeout_success_handler(
    holder: Holder,
) -> impl for<'a> Fn(&'a HotlinePreamble, &'a mut TcpStream) -> BoxFuture<'a, io::Result<()>>
+ Send
+ Sync
+ 'static {
    move |p, stream| {
        let holder = holder.clone();
        let clone = p.clone();
        Box::pin(async move {
            notify_holder(&holder)?;
            stream.write_all(b"OK").await?;
            stream.flush().await?;
            // keep connection open by not shutting down here
            if clone.magic != HotlinePreamble::MAGIC {
                return Err(io::Error::other(format!(
                    "preamble magic mismatch: expected {:?}, got {:?}",
                    HotlinePreamble::MAGIC,
                    clone.magic
                )));
            }
            Ok::<(), io::Error>(())
        })
    }
}

/// A valid, promptly delivered `HotlinePreamble` payload.
const PROMPT_PREAMBLE: &[u8] = b"TRTPHOTL\x00\x01\x00\x02";

/// Build a failure callback that requires a timed-out decode error.
///
/// The callback optionally replies before shutting the stream down, then
/// signals through the shared holder.
fn timed_out_failure_handler(
    holder: Holder,
    reply: Option<&'static [u8]>,
) -> impl for<'a> Fn(&'a DecodeError, &'a mut TcpStream) -> BoxFuture<'a, io::Result<()>>
+ Send
+ Sync
+ 'static {
    move |err, stream| {
        let holder = holder.clone();
        Box::pin(async move {
            ensure_timed_out_decode_error(err)?;
            if let Some(reply) = reply {
                stream.write_all(reply).await?;
                stream.flush().await?;
            }
            stream.shutdown().await?;
            notify_holder(&holder)?;
            Ok::<(), io::Error>(())
        })
    }
}

/// Read one byte and report whether the peer closed the connection.
async fn expect_closed(stream: &mut TcpStream, limit: Duration) -> TestResult<bool> {
    let mut eof = [0u8; 1];
    let read = timeout(limit, stream.read(&mut eof)).await;
    is_connection_closed(read?)
}

/// Drive a server whose deadline must expire and assert the outcome.
///
/// The failure callback must observe a timed-out decode error, optionally
/// reply, and close the connection. `minimum_elapsed` guards the tests that
/// assert the server waited for its configured deadline before acting, and
/// `observation_limit` bounds how long that wait may take. A limit shorter
/// than the default deadline makes the test fail when a shorter override is
/// ignored.
async fn assert_timed_out_connection<F>(
    server: WireframeServer<F, HotlinePreamble>,
    minimum_elapsed: Duration,
    observation_limit: Duration,
    reply: Option<&'static [u8]>,
) -> TestResult
where
    F: Fn() -> WireframeApp + Send + Sync + Clone + 'static,
{
    let (failure_holder, failure_rx) = channel_holder();
    let (result_tx, result_rx) = oneshot::channel();
    let server =
        server.on_preamble_decode_failure(timed_out_failure_handler(failure_holder, reply));

    with_running_server(server, move |addr| async move {
        // Start the clock before connecting so the measured interval covers
        // the whole wait, including the server's accept and timer start.
        let started = Instant::now();
        let mut stream = TcpStream::connect(addr).await?;
        // The test-side bound outlives the server deadline so paused time
        // auto-advances to the server's timer first.
        recv_within(observation_limit, failure_rx).await?;
        let elapsed = started.elapsed();

        let reply_bytes = if let Some(expected) = reply {
            let mut buf = vec![0u8; expected.len()];
            timeout(
                Duration::from_millis(READ_TIMEOUT_MS),
                stream.read_exact(&mut buf),
            )
            .await??;
            Some(buf)
        } else {
            None
        };
        let closed = expect_closed(&mut stream, Duration::from_millis(200)).await?;
        let _ = result_tx.send((elapsed, closed, reply_bytes));
        Ok(())
    })
    .await?;

    let (elapsed, closed, reply_bytes) = recv_within(Duration::from_secs(1), result_rx).await?;
    if !closed {
        return Err("expected the connection to close after a timed-out preamble".into());
    }
    if elapsed < minimum_elapsed {
        return Err(format!("failure fired before the deadline: {elapsed:?}").into());
    }
    if let Some(expected) = reply {
        let actual = reply_bytes
            .as_deref()
            .ok_or_else(|| io::Error::other("failure callback sent no reply"))?;
        if actual != expected {
            return Err(format!("response mismatch: expected {expected:?}, got {actual:?}").into());
        }
    }
    Ok(())
}

/// Drive a server with a prompt preamble and assert the success path.
async fn assert_prompt_preamble_succeeds<F>(
    server: WireframeServer<F, HotlinePreamble>,
) -> TestResult
where
    F: Fn() -> WireframeApp + Send + Sync + Clone + 'static,
{
    let (success_holder, success_rx) = channel_holder();
    let (failure_holder, failure_rx) = channel_holder();
    let server = server
        .on_preamble_decode_success(timeout_success_handler(success_holder))
        .on_preamble_decode_failure(failure_cb(failure_holder));

    with_running_server(server, move |addr| async move {
        let mut stream = TcpStream::connect(addr).await?;
        stream.write_all(PROMPT_PREAMBLE).await?;

        recv_within(Duration::from_millis(200), success_rx).await?;
        let failure_fired = timeout(Duration::from_millis(150), failure_rx)
            .await
            .is_ok();

        let mut buf = [0u8; 2];
        timeout(
            Duration::from_millis(READ_TIMEOUT_MS),
            stream.read_exact(&mut buf),
        )
        .await??;
        if buf.as_slice() != b"OK" {
            return Err(format!("response mismatch: expected b\"OK\", got {buf:?}").into());
        }
        if failure_fired {
            return Err("failure handler should not fire for timely preamble".into());
        }
        Ok(())
    })
    .await
}

#[tokio::test]
async fn preamble_timeout_invokes_failure_handler_and_closes_connection() -> TestResult {
    let server = WireframeServer::new(factory())
        .with_preamble::<HotlinePreamble>()
        .preamble_timeout(Duration::from_millis(50));
    assert_timed_out_connection(
        server,
        Duration::from_millis(50),
        Duration::from_secs(5),
        Some(b"ERR"),
    )
    .await
}

#[tokio::test]
async fn preamble_timeout_allows_timely_preamble() -> TestResult {
    let server = WireframeServer::new(factory())
        .with_preamble::<HotlinePreamble>()
        .preamble_timeout(Duration::from_millis(150));
    assert_prompt_preamble_succeeds(server).await
}

/// A default server closes an idle connection after the default deadline.
#[tokio::test(start_paused = true)]
async fn default_preamble_timeout_closes_idle_unit_preamble_connection() -> TestResult {
    let server = WireframeServer::new(factory());
    let (result_tx, result_rx) = oneshot::channel();
    with_running_server(server, move |addr| async move {
        let mut stream = TcpStream::connect(addr).await?;
        let started = Instant::now();
        let mut buf = [0u8; 1];
        // The test-side bound outlives the server deadline so paused time
        // auto-advances to the server's timer first.
        let read = timeout(beyond_default_timeout(), stream.read(&mut buf)).await;
        let closed = is_connection_closed(read?)?;
        let elapsed = started.elapsed();
        let _ = result_tx.send((closed, elapsed));
        Ok(())
    })
    .await?;

    let (closed, elapsed) = recv_within(beyond_default_timeout(), result_rx).await?;
    if !closed {
        return Err("expected the default deadline to close the idle connection".into());
    }
    if elapsed < DEFAULT_PREAMBLE_TIMEOUT {
        return Err(format!("connection closed before the default deadline: {elapsed:?}").into());
    }
    Ok(())
}

/// The default deadline reaches non-unit preambles and reports `TimedOut`.
#[tokio::test(start_paused = true)]
async fn default_preamble_timeout_reports_timed_out_decode_error() -> TestResult {
    let server = WireframeServer::new(factory()).with_preamble::<HotlinePreamble>();
    assert_timed_out_connection(
        server,
        DEFAULT_PREAMBLE_TIMEOUT,
        beyond_default_timeout(),
        None,
    )
    .await
}

/// Disabling the deadline keeps an idle connection open indefinitely.
#[tokio::test(start_paused = true)]
async fn disabled_preamble_timeout_keeps_idle_connection_open() -> TestResult {
    let factory = factory();
    let (failure_holder, mut failure_rx) = channel_holder();
    let (result_tx, result_rx) = oneshot::channel();
    let server = WireframeServer::new(factory)
        .with_preamble::<HotlinePreamble>()
        .disable_preamble_timeout()
        .on_preamble_decode_failure(failure_cb(failure_holder));

    with_running_server(server, move |addr| async move {
        let mut stream = TcpStream::connect(addr).await?;
        let mut buf = [0u8; 1];
        // The only timer is the test-side one, so paused time advances well
        // past the default deadline without the server closing the socket.
        let read = timeout(beyond_default_timeout(), stream.read(&mut buf)).await;
        let still_open = read.is_err();
        let failure_fired = failure_rx.try_recv().is_ok();
        // Closing here lets the fixture's graceful drain finish.
        drop(stream);
        let _ = result_tx.send((still_open, failure_fired));
        Ok(())
    })
    .await?;

    let (still_open, failure_fired) = recv_within(Duration::from_secs(1), result_rx).await?;
    if !still_open {
        return Err("opt-out server closed an idle connection before the test deadline".into());
    }
    if failure_fired {
        return Err("opt-out server ran the failure callback for an idle connection".into());
    }
    Ok(())
}

/// A default deadline still accepts a prompt preamble.
#[tokio::test]
async fn default_preamble_timeout_allows_timely_preamble() -> TestResult {
    let server = WireframeServer::new(factory()).with_preamble::<HotlinePreamble>();
    assert_prompt_preamble_succeeds(server).await
}
