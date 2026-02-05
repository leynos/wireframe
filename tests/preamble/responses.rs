//! Response-writing tests for preamble handlers.

use std::io;

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::oneshot,
    time::{Duration, timeout},
};
use wireframe::server::WireframeServer;
use wireframe_testing::{TestResult, factory};

use crate::support::{
    HotlinePreamble,
    channel_holder,
    notify_holder,
    recv_within,
    server_with_handlers,
    with_running_server,
};

#[tokio::test]
#[expect(
    clippy::panic_in_result_fn,
    reason = "asserts provide clearer diagnostics in tests"
)]
async fn success_callback_can_write_response() -> TestResult {
    let factory = factory();
    let (response_tx, response_rx) = oneshot::channel();
    let server = server_with_handlers(
        factory,
        |_, stream| {
            Box::pin(async move {
                stream.write_all(b"ACK").await?;
                stream.flush().await?;
                Ok::<(), io::Error>(())
            })
        },
        |_, _| Box::pin(async { Ok::<(), io::Error>(()) }),
    );

    with_running_server(server, move |addr| async move {
        let mut stream = TcpStream::connect(addr).await?;
        let bytes = b"TRTPHOTL\x00\x01\x00\x02";
        stream.write_all(bytes).await?;
        let mut buf = [0u8; 3];
        stream.read_exact(&mut buf).await?;
        let _ = response_tx.send(buf);
        Ok(())
    })
    .await?;
    let buf = recv_within(Duration::from_secs(1), response_rx).await?;
    assert_eq!(&buf, b"ACK");
    Ok(())
}

#[tokio::test]
#[expect(
    clippy::panic_in_result_fn,
    reason = "asserts provide clearer diagnostics in tests"
)]
async fn failure_callback_can_write_response() -> TestResult {
    let factory = factory();
    let (failure_holder, failure_rx) = channel_holder();
    let (response_tx, response_rx) = oneshot::channel();
    let server = WireframeServer::new(factory)
        .with_preamble::<HotlinePreamble>()
        .on_preamble_decode_failure(move |_, stream| {
            let failure_holder = failure_holder.clone();
            Box::pin(async move {
                stream.write_all(b"ERR").await?;
                stream.flush().await?;
                notify_holder(&failure_holder)?;
                Ok::<(), io::Error>(())
            })
        });

    with_running_server(server, move |addr| async move {
        let mut stream = TcpStream::connect(addr).await?;
        stream.write_all(b"BAD").await?;
        stream.shutdown().await?;
        let mut buf = [0u8; 3];
        let read = timeout(Duration::from_secs(1), stream.read_exact(&mut buf)).await;
        let result = read?;
        result?;
        let _ = response_tx.send(buf);
        Ok(())
    })
    .await?;
    let buf = recv_within(Duration::from_secs(1), response_rx).await?;
    assert_eq!(&buf, b"ERR");
    recv_within(Duration::from_millis(200), failure_rx).await?;
    Ok(())
}
