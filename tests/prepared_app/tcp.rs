//! Real-socket coverage for prepared applications.
//!
//! These scenarios drive a prepared application over a loopback listener so
//! that dispatch, middleware ordering, and transport reuse are exercised
//! through the socket path rather than the in-process frame drivers.

use std::{net::SocketAddr, time::Duration};

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    time::timeout,
};
use wireframe_testing::TestResult;

use super::{
    ConnectionStartupCounts,
    ConnectionStartupInstrumentation,
    MIDDLEWARE_LAYERS,
    ROUTES,
    TestPreparedApp,
    build_frame,
    counted_app_factory,
    fallible_check_equal::check_equal,
    response_payload,
};

/// Bounded waits keep the real-socket test from hanging on a stalled peer.
const TCP_OPERATION_TIMEOUT: Duration = Duration::from_secs(5);

/// Accepts one TCP connection and processes it with the prepared application.
async fn serve_one_prepared_tcp_connection(
    listener: TcpListener,
    prepared: TestPreparedApp,
) -> TestResult<()> {
    let (stream, _peer) = timeout(TCP_OPERATION_TIMEOUT, listener.accept())
        .await
        .map_err(|_| "prepared TCP accept timed out".to_string())??;
    timeout(
        TCP_OPERATION_TIMEOUT,
        prepared.handle_connection_result(stream),
    )
    .await
    .map_err(|_| "prepared TCP connection processing timed out".to_string())??;
    Ok(())
}

/// Sends one encoded frame to the server and returns the decoded response.
async fn exchange_one_tcp_frame(address: SocketAddr, frame: Vec<u8>) -> TestResult<Vec<u8>> {
    let mut client = timeout(TCP_OPERATION_TIMEOUT, TcpStream::connect(address))
        .await
        .map_err(|_| "prepared TCP client connect timed out".to_string())??;
    client.write_all(&frame).await?;
    // Signal end-of-input so the server can finish the request-response cycle.
    client.shutdown().await?;
    let mut response = Vec::new();
    timeout(TCP_OPERATION_TIMEOUT, client.read_to_end(&mut response))
        .await
        .map_err(|_| "prepared TCP response read timed out".to_string())??;
    response_payload(&response)
}

#[tokio::test]
async fn prepared_app_serves_real_tcp_connection_without_retransforming() -> TestResult<()> {
    let instrumentation = ConnectionStartupInstrumentation::new();
    let prepared: TestPreparedApp = counted_app_factory(instrumentation.clone())()?
        .prepare()
        .await
        .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> { Box::new(error) })?;
    let prepared_counts = ConnectionStartupCounts {
        factory_calls: 1,
        transforms: ROUTES * MIDDLEWARE_LAYERS,
    };
    check_equal(
        &instrumentation.snapshot(),
        &prepared_counts,
        "preparation should transform each route once",
    )?;

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let server = tokio::spawn(serve_one_prepared_tcp_connection(listener, prepared));

    let payload = exchange_one_tcp_frame(address, build_frame(1, vec![b'X'])?).await?;
    // The request gains tags A then B in wrap order; the response unwinds them.
    check_equal(
        &payload,
        b"XABBA",
        "the real TCP response should follow middleware order",
    )?;

    let joined = timeout(TCP_OPERATION_TIMEOUT, server)
        .await
        .map_err(|_| "prepared TCP accept task did not finish".to_string())?;
    // Surfaces both a failed join and a failed connection-processing result.
    joined??;
    // Preparation transformed each route once; the connection added none.
    check_equal(
        &instrumentation.snapshot(),
        &prepared_counts,
        "the real TCP connection should not add transforms",
    )?;
    Ok(())
}
