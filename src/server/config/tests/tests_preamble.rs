//! Preamble configuration tests for `WireframeServer`.

use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use bincode::error::DecodeError;
use rstest::rstest;
use tokio::net::{TcpListener, TcpStream};

use super::PreambleHandlerKind;
use crate::{
    app::WireframeApp,
    server::{
        WireframeServer,
        test_util::{TestPreamble, factory, server_with_preamble},
    },
};

#[rstest]
fn test_with_preamble_type_conversion(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
) {
    let server = WireframeServer::new(factory).with_preamble::<TestPreamble>();
    assert_eq!(
        server.worker_count(),
        super::expected_default_worker_count()
    );
}

#[rstest]
fn test_preamble_timeout_configuration(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
) {
    let timeout = Duration::from_millis(25);
    let server = WireframeServer::new(factory.clone()).preamble_timeout(timeout);
    assert_eq!(server.preamble_timeout, Some(timeout));

    let clamped = WireframeServer::new(factory).preamble_timeout(Duration::ZERO);
    assert_eq!(clamped.preamble_timeout, Some(Duration::from_millis(1)));
}

#[rstest]
#[case::success(PreambleHandlerKind::Success)]
#[case::failure(PreambleHandlerKind::Failure)]
#[tokio::test]
async fn test_preamble_handler_registration(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    #[case] handler: PreambleHandlerKind,
) {
    let counter = Arc::new(AtomicUsize::new(0));
    let c = counter.clone();

    let server = server_with_preamble(factory);
    let server = match handler {
        PreambleHandlerKind::Success => {
            server.on_preamble_decode_success(move |_p: &TestPreamble, _| {
                let c = c.clone();
                Box::pin(async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                })
            })
        }
        PreambleHandlerKind::Failure => {
            server.on_preamble_decode_failure(move |_err: &DecodeError, _stream| {
                let c = c.clone();
                Box::pin(async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    Ok::<(), io::Error>(())
                })
            })
        }
    };

    assert_eq!(counter.load(Ordering::SeqCst), 0);
    match handler {
        PreambleHandlerKind::Success => {
            assert!(server.on_preamble_success.is_some());
            let handler = server
                .on_preamble_success
                .as_ref()
                .expect("success handler missing");
            let listener = TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind listener");
            let addr = listener.local_addr().expect("listener addr");
            let _client = TcpStream::connect(addr)
                .await
                .expect("client connect failed");
            let (mut stream, _) = listener.accept().await.expect("accept stream");
            let preamble = TestPreamble {
                id: 0,
                message: String::new(),
            };
            handler(&preamble, &mut stream)
                .await
                .expect("handler failed");
        }
        PreambleHandlerKind::Failure => {
            assert!(server.on_preamble_failure.is_some());
            let handler = server
                .on_preamble_failure
                .as_ref()
                .expect("failure handler missing");
            let listener = TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind listener");
            let addr = listener.local_addr().expect("listener addr");
            let _client = TcpStream::connect(addr)
                .await
                .expect("client connect failed");
            let (mut stream, _) = listener.accept().await.expect("accept stream");
            handler(&DecodeError::UnexpectedEnd { additional: 0 }, &mut stream)
                .await
                .expect("handler failed");
        }
    }
    assert_eq!(counter.load(Ordering::SeqCst), 1);
}
