//! Callback behaviour tests for preamble handling.

use std::{io, sync::Arc};

use rstest::rstest;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    time::{Duration, timeout},
};
use wireframe::{app::WireframeApp, server::WireframeServer};
use wireframe_testing::{TestResult, factory};

use crate::support::{
    HotlinePreamble,
    OtherPreamble,
    channel_holder,
    failure_cb,
    recv_within,
    server_with_handlers,
    success_cb,
    take_sender_io,
    with_running_server,
};

#[derive(Clone, Copy)]
enum ExpectedCallback {
    Success,
    Failure,
}

#[rstest]
#[case(b"TRTPHOTL\x00\x01\x00\x02", ExpectedCallback::Success)]
#[case(b"TRTPHOT", ExpectedCallback::Failure)]
#[tokio::test]
async fn server_triggers_expected_callback(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    #[case] bytes: &'static [u8],
    #[case] expected: ExpectedCallback,
) -> TestResult {
    let (success_tx, success_rx) = tokio::sync::oneshot::channel::<HotlinePreamble>();
    let (failure_tx, failure_rx) = tokio::sync::oneshot::channel::<()>();
    let success_tx = Arc::new(std::sync::Mutex::new(Some(success_tx)));
    let failure_tx = Arc::new(std::sync::Mutex::new(Some(failure_tx)));
    let server = server_with_handlers(
        factory,
        {
            let success_tx = success_tx.clone();
            move |p, _| {
                let success_tx = success_tx.clone();
                let clone = p.clone();
                Box::pin(async move {
                    if let Some(tx) = take_sender_io(&success_tx)? {
                        let _ = tx.send(clone);
                    }
                    Ok::<(), io::Error>(())
                })
            }
        },
        {
            let failure_tx = failure_tx.clone();
            move |_, _| {
                let failure_tx = failure_tx.clone();
                Box::pin(async move {
                    if let Some(tx) = take_sender_io(&failure_tx)? {
                        let _ = tx.send(());
                    }
                    Ok::<(), io::Error>(())
                })
            }
        },
    );

    with_running_server(server, |addr| async move {
        let mut stream = TcpStream::connect(addr).await?;
        stream.write_all(bytes).await?;
        stream.shutdown().await?;

        match expected {
            ExpectedCallback::Success => {
                let preamble = timeout(Duration::from_secs(2), success_rx).await??;
                assert_eq!(preamble.magic, HotlinePreamble::MAGIC);
                assert!(timeout(Duration::from_secs(1), failure_rx).await.is_err());
            }
            ExpectedCallback::Failure => {
                timeout(Duration::from_secs(2), failure_rx).await??;
                assert!(timeout(Duration::from_secs(1), success_rx).await.is_err());
            }
        }

        Ok(())
    })
    .await?;
    Ok(())
}

#[rstest]
#[tokio::test]
async fn success_handler_runs_without_failure_handler(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
) -> TestResult {
    let (success_tx, success_rx) = tokio::sync::oneshot::channel::<HotlinePreamble>();
    let success_tx = Arc::new(std::sync::Mutex::new(Some(success_tx)));
    let server = WireframeServer::new(factory)
        .with_preamble::<HotlinePreamble>()
        .on_preamble_decode_success({
            let success_tx = success_tx.clone();
            move |p, _| {
                let success_tx = success_tx.clone();
                let preamble = p.clone();
                Box::pin(async move {
                    if let Some(tx) = take_sender_io(&success_tx)? {
                        let _ = tx.send(preamble);
                    }
                    Ok::<(), io::Error>(())
                })
            }
        });

    with_running_server(server, |addr| async move {
        let mut stream = TcpStream::connect(addr).await?;
        let bytes = b"TRTPHOTL\x00\x01\x00\x02";
        stream.write_all(bytes).await?;
        stream.shutdown().await?;
        let preamble = recv_within(Duration::from_secs(1), success_rx).await?;
        assert_eq!(preamble.magic, HotlinePreamble::MAGIC);
        Ok(())
    })
    .await?;
    Ok(())
}

#[rstest]
#[tokio::test]
async fn failure_handler_error_is_logged_and_connection_closes(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
) -> TestResult {
    let (failure_holder, failure_rx) = channel_holder();
    let server = WireframeServer::new(factory)
        .with_preamble::<HotlinePreamble>()
        .on_preamble_decode_failure(move |_, _| {
            let failure_holder = failure_holder.clone();
            Box::pin(async move {
                if let Some(tx) = take_sender_io(&failure_holder)? {
                    let _ = tx.send(());
                }
                Err::<(), io::Error>(io::Error::other("boom"))
            })
        });

    with_running_server(server, |addr| async move {
        let mut stream = TcpStream::connect(addr).await?;
        stream.write_all(b"BAD").await?;
        stream.shutdown().await?;

        recv_within(Duration::from_secs(1), failure_rx).await?;

        let mut buf = [0u8; 1];
        let read = timeout(Duration::from_millis(200), stream.read(&mut buf)).await;
        match read? {
            Ok(0) => {}
            Ok(n) => panic!("expected connection close, read {n} bytes"),
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
async fn callbacks_dropped_when_overriding_preamble(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
) -> TestResult {
    let (hotline_success, hotline_success_rx) = channel_holder();
    let (hotline_failure, hotline_failure_rx) = channel_holder();
    let (other_success, other_success_rx) = channel_holder();
    let (other_failure, other_failure_rx) = channel_holder();

    let server = WireframeServer::new(factory.clone())
        .with_preamble::<HotlinePreamble>()
        .on_preamble_decode_success(success_cb::<HotlinePreamble>(hotline_success.clone()))
        .on_preamble_decode_failure(failure_cb(hotline_failure.clone()))
        .with_preamble::<OtherPreamble>()
        .on_preamble_decode_success(success_cb::<OtherPreamble>(other_success.clone()))
        .on_preamble_decode_failure(failure_cb(other_failure.clone()));

    with_running_server(server, |addr| async move {
        let mut stream = TcpStream::connect(addr).await?;
        let config = bincode::config::standard()
            .with_big_endian()
            .with_fixed_int_encoding();
        let mut bytes = bincode::encode_to_vec(OtherPreamble(1), config)?;
        bytes.resize(8, 0);
        stream.write_all(&bytes).await?;
        stream.shutdown().await?;
        // Wait for the success callback before shutting down the server.
        recv_within(Duration::from_secs(1), other_success_rx).await?;
        Ok(())
    })
    .await?;
    assert!(
        timeout(Duration::from_millis(500), other_failure_rx)
            .await
            .is_err(),
        "other failure callback invoked",
    );
    assert!(
        timeout(Duration::from_millis(500), hotline_success_rx)
            .await
            .is_err(),
        "hotline success callback invoked",
    );
    assert!(
        timeout(Duration::from_millis(500), hotline_failure_rx)
            .await
            .is_err(),
        "hotline failure callback invoked",
    );
    Ok(())
}
