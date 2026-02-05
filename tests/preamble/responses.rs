//! Response-writing tests for preamble handlers.

use std::io;

use rstest::rstest;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    time::{Duration, timeout},
};
use wireframe::{app::WireframeApp, server::WireframeServer};

use crate::{
    common::{TestResult, factory},
    support::{
        HotlinePreamble,
        channel_holder,
        recv_within,
        server_with_handlers,
        take_sender_io,
        with_running_server,
    },
};

#[rstest]
#[tokio::test]
async fn success_callback_can_write_response(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
) -> TestResult {
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

    with_running_server(server, |addr| async move {
        let mut stream = TcpStream::connect(addr).await?;
        let bytes = b"TRTPHOTL\x00\x01\x00\x02";
        stream.write_all(bytes).await?;
        let mut buf = [0u8; 3];
        stream.read_exact(&mut buf).await?;
        assert_eq!(&buf, b"ACK");
        Ok(())
    })
    .await?;
    Ok(())
}

#[rstest]
#[tokio::test]
async fn failure_callback_can_write_response(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
) -> TestResult {
    let (failure_holder, failure_rx) = channel_holder();
    let server = WireframeServer::new(factory)
        .with_preamble::<HotlinePreamble>()
        .on_preamble_decode_failure(move |_, stream| {
            let failure_holder = failure_holder.clone();
            Box::pin(async move {
                stream.write_all(b"ERR").await?;
                stream.flush().await?;
                if let Some(tx) = take_sender_io(&failure_holder)? {
                    let _ = tx.send(());
                }
                Ok::<(), io::Error>(())
            })
        });

    with_running_server(server, |addr| async move {
        let mut stream = TcpStream::connect(addr).await?;
        stream.write_all(b"BAD").await?;
        stream.shutdown().await?;
        let mut buf = [0u8; 3];
        let read = timeout(Duration::from_secs(1), stream.read_exact(&mut buf)).await;
        let result = read?;
        result?;
        assert_eq!(&buf, b"ERR");
        recv_within(Duration::from_millis(200), failure_rx).await?;
        Ok(())
    })
    .await?;
    Ok(())
}
