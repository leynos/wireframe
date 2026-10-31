//! Preamble configuration tests for `WireframeServer`.

use std::{
    io,
    net::TcpListener as StdTcpListener,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use bincode::error::DecodeError;
use rstest::{fixture, rstest};
use tokio::net::{TcpListener, TcpStream};

use super::PreambleHandlerKind;
use crate::{
    app::WireframeApp,
    server::{
        DEFAULT_PREAMBLE_TIMEOUT,
        WireframeServer,
        default_worker_count,
        test_util::{TestPreamble, factory, server_with_preamble},
    },
};

#[rstest]
fn test_with_preamble_type_conversion(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
) {
    let server = WireframeServer::new(factory).with_preamble::<TestPreamble>();
    assert_eq!(server.worker_count(), default_worker_count());
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
fn test_default_preamble_timeout(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
) {
    let from_factory = WireframeServer::new(factory);
    assert_eq!(
        from_factory.preamble_timeout,
        Some(DEFAULT_PREAMBLE_TIMEOUT)
    );

    let from_app: WireframeServer<_> = WireframeServer::from_app(WireframeApp::default());
    assert_eq!(from_app.preamble_timeout, Some(DEFAULT_PREAMBLE_TIMEOUT));
}

#[rstest]
fn test_disable_preamble_timeout(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
) {
    let timeout = Duration::from_millis(25);

    let opted_out = WireframeServer::new(factory.clone()).disable_preamble_timeout();
    assert_eq!(opted_out.preamble_timeout, None);

    // The last call wins: a setter re-enables the deadline after an opt-out.
    let reenabled = WireframeServer::new(factory.clone())
        .disable_preamble_timeout()
        .preamble_timeout(timeout);
    assert_eq!(reenabled.preamble_timeout, Some(timeout));

    let disabled_after_setter = WireframeServer::new(factory)
        .preamble_timeout(timeout)
        .disable_preamble_timeout();
    assert_eq!(disabled_after_setter.preamble_timeout, None);
}

#[rstest]
fn test_with_preamble_preserves_timeout_configuration(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
) {
    let timeout = Duration::from_millis(25);

    let default = WireframeServer::new(factory.clone()).with_preamble::<TestPreamble>();
    assert_eq!(default.preamble_timeout, Some(DEFAULT_PREAMBLE_TIMEOUT));

    let custom = WireframeServer::new(factory.clone())
        .preamble_timeout(timeout)
        .with_preamble::<TestPreamble>();
    assert_eq!(custom.preamble_timeout, Some(timeout));

    let opted_out = WireframeServer::new(factory)
        .disable_preamble_timeout()
        .with_preamble::<TestPreamble>();
    assert_eq!(opted_out.preamble_timeout, None);
}

// `bind_existing_listener` converts the std listener with
// `TcpListener::from_std`, which requires a Tokio runtime context.
#[rstest]
#[tokio::test]
async fn test_binding_preserves_timeout_configuration(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
) -> io::Result<()> {
    let timeout = Duration::from_millis(25);
    let listener = || StdTcpListener::bind("127.0.0.1:0");

    let default = WireframeServer::new(factory.clone())
        .bind_existing_listener(listener()?)
        .expect("bind default server");
    assert_eq!(default.preamble_timeout, Some(DEFAULT_PREAMBLE_TIMEOUT));

    let rebound = default
        .bind("127.0.0.1:0".parse().expect("parse rebind address"))
        .expect("rebind default server");
    assert_eq!(rebound.preamble_timeout, Some(DEFAULT_PREAMBLE_TIMEOUT));

    let custom = WireframeServer::new(factory.clone())
        .preamble_timeout(timeout)
        .bind_existing_listener(listener()?)
        .expect("bind custom server");
    assert_eq!(custom.preamble_timeout, Some(timeout));

    let opted_out = WireframeServer::new(factory)
        .disable_preamble_timeout()
        .bind_existing_listener(listener()?)
        .expect("bind opted-out server");
    assert_eq!(opted_out.preamble_timeout, None);
    Ok(())
}

#[rstest]
#[case::success(PreambleHandlerKind::Success)]
#[case::failure(PreambleHandlerKind::Failure)]
#[tokio::test]
async fn test_preamble_handler_registration(
    factory: impl Fn() -> WireframeApp + Send + Sync + Clone + 'static,
    #[case] handler: PreambleHandlerKind,
    bound_listener: io::Result<StdTcpListener>,
) -> io::Result<()> {
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
    let mut stream = accept_stream(bound_listener?).await?;
    match handler {
        PreambleHandlerKind::Success => {
            let handler = server
                .on_preamble_success
                .as_ref()
                .expect("success handler missing");
            let preamble = TestPreamble {
                id: 0,
                message: String::new(),
            };
            handler(&preamble, &mut stream).await?;
        }
        PreambleHandlerKind::Failure => {
            let handler = server
                .on_preamble_failure
                .as_ref()
                .expect("failure handler missing");
            handler(&DecodeError::UnexpectedEnd { additional: 0 }, &mut stream).await?;
        }
    }
    assert_eq!(counter.load(Ordering::SeqCst), 1);
    Ok(())
}

#[fixture]
fn bound_listener() -> io::Result<StdTcpListener> {
    let addr = "127.0.0.1:0";
    StdTcpListener::bind(addr)
}

async fn accept_stream(listener: StdTcpListener) -> io::Result<TcpStream> {
    let addr = listener.local_addr()?;
    listener.set_nonblocking(true)?;
    let listener = TcpListener::from_std(listener)?;
    let _client = TcpStream::connect(addr).await?;
    let (stream, _) = listener.accept().await?;
    Ok(stream)
}
