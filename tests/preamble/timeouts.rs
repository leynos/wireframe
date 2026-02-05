//! Timeout behaviour tests for preamble handling.

use std::io;

use bincode::error::DecodeError;
use rstest::rstest;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    time::{Duration, timeout},
};
use wireframe::{app::WireframeApp, server::WireframeServer};

use crate::{
    common::{TestResult, factory},
    support::{HotlinePreamble, channel_holder, recv_within, take_sender_io, with_running_server},
};

#[rstest]
#[tokio::test]
async fn preamble_timeout_invokes_failure_handler_and_closes_connection(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
) -> TestResult {
    let (failure_holder, failure_rx) = channel_holder();
    let server = WireframeServer::new(factory)
        .with_preamble::<HotlinePreamble>()
        .preamble_timeout(Duration::from_millis(50))
        .on_preamble_decode_failure(move |err, stream| {
            let failure_holder = failure_holder.clone();
            Box::pin(async move {
                assert!(
                    matches!(
                        err,
                        DecodeError::Io { inner, .. }
                            if inner.kind() == io::ErrorKind::TimedOut
                    ),
                    "expected timed out error, got {err:?}"
                );
                stream.write_all(b"ERR").await?;
                stream.flush().await?;
                stream.shutdown().await?;
                if let Some(tx) = take_sender_io(&failure_holder)? {
                    let _ = tx.send(());
                }
                Ok::<(), io::Error>(())
            })
        });

    with_running_server(server, |addr| async move {
        let mut stream = TcpStream::connect(addr).await?;
        recv_within(Duration::from_secs(1), failure_rx).await?;
        let mut buf = [0u8; 3];
        timeout(Duration::from_millis(500), stream.read_exact(&mut buf)).await??;
        assert_eq!(&buf, b"ERR");
        let mut eof = [0u8; 1];
        let read = timeout(Duration::from_millis(200), stream.read(&mut eof)).await;
        match read? {
            Ok(0) => {}
            Ok(n) => panic!("expected connection to close, read {n} bytes"),
            Err(e) if e.kind() == io::ErrorKind::ConnectionReset => {}
            Err(e) => panic!("unexpected read error: {e:?}"),
        }
        Ok(())
    })
    .await?;
    Ok(())
}

#[rstest]
#[tokio::test]
async fn preamble_timeout_allows_timely_preamble(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
) -> TestResult {
    let (success_holder, success_rx) = channel_holder();
    let (failure_holder, failure_rx) = channel_holder();
    let server = WireframeServer::new(factory)
        .with_preamble::<HotlinePreamble>()
        .preamble_timeout(Duration::from_millis(150))
        .on_preamble_decode_success({
            let success_holder = success_holder.clone();
            move |p, stream| {
                let success_holder = success_holder.clone();
                let clone = p.clone();
                Box::pin(async move {
                    if let Some(tx) = take_sender_io(&success_holder)? {
                        let _ = tx.send(());
                    }
                    stream.write_all(b"OK").await?;
                    stream.flush().await?;
                    // keep connection open by not shutting down here
                    assert_eq!(clone.magic, HotlinePreamble::MAGIC);
                    Ok::<(), io::Error>(())
                })
            }
        })
        .on_preamble_decode_failure({
            let failure_holder = failure_holder.clone();
            move |_, _| {
                let failure_holder = failure_holder.clone();
                Box::pin(async move {
                    if let Some(tx) = take_sender_io(&failure_holder)? {
                        let _ = tx.send(());
                    }
                    Ok::<(), io::Error>(())
                })
            }
        });

    with_running_server(server, |addr| async move {
        let mut stream = TcpStream::connect(addr).await?;
        let bytes = b"TRTPHOTL\x00\x01\x00\x02";
        stream.write_all(bytes).await?;

        recv_within(Duration::from_millis(200), success_rx).await?;
        assert!(
            timeout(Duration::from_millis(150), failure_rx)
                .await
                .is_err(),
            "failure handler should not fire for timely preamble"
        );

        let mut buf = [0u8; 2];
        stream.read_exact(&mut buf).await?;
        assert_eq!(&buf, b"OK");
        Ok(())
    })
    .await?;
    Ok(())
}
