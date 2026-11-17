//! A server stop must not wait for a connected peer that sends nothing.
//!
//! An idle peer keeps its socket open, so the inbound read never reaches
//! end-of-file and a read timeout only retries it. The connection task, its
//! gauge guard and `ServerShutdown::drained` would then stay pending for as
//! long as the peer stays connected. Observing the shutdown token around the
//! inbound read is what lets the drain finish.
#![cfg(not(loom))]
#![forbid(unsafe_code)]

use std::{sync::Arc, time::Duration};

use bytes::Bytes;
use futures::StreamExt;
use tokio::{io::AsyncWriteExt, net::TcpStream, sync::oneshot, time::timeout};
use tokio_util::codec::Framed;
use wireframe::{
    app::{Envelope, ResponseHandler, WireframeApp},
    codec::{FrameCodec, LengthDelimitedFrameCodec},
    response::Response,
    serializer::{BincodeSerializer, Serializer},
    server::WireframeServer,
};
use wireframe_testing::{
    TestError,
    TestResult,
    encode_frame,
    unused_listener,
    wait_for_server_readiness,
};

/// Application type shared by the request builder and the server under test.
type TestApp = WireframeApp<BincodeSerializer, (), Envelope>;

/// Route that answers immediately, leaving the connection idle afterwards.
const ECHO_ROUTE: u32 = 61;
/// Payload the handler replies with.
const REPLY: u8 = 9;
/// Maximum frame length used by the in-memory transport.
const MAX_FRAME_LENGTH: usize = 2048;
/// Upper bound for the reply and for the drain that follows the stop.
const STEP_TIMEOUT: Duration = Duration::from_secs(20);

/// Require `condition`, reporting `message` as the test failure otherwise.
fn ensure(condition: bool, message: impl Into<String>) -> TestResult {
    if condition {
        Ok(())
    } else {
        Err(TestError::Msg(message.into()))
    }
}

/// Handle a request with one complete response frame.
fn echo_handler() -> ResponseHandler<Envelope> {
    Arc::new(|_request: &Envelope| {
        Box::pin(async { Response::Single(Envelope::new(ECHO_ROUTE, None, vec![REPLY])) })
    })
}

/// Encode an empty request for `route` in the transport's frame format.
fn build_request(route: u32) -> TestResult<Vec<u8>> {
    let encoded = BincodeSerializer.serialize(&Envelope::new(route, Some(7), Vec::new()))?;
    let mut codec = TestApp::default().length_codec();
    Ok(encode_frame(&mut codec, encoded)?)
}

/// A stop must drain a connection whose peer is still connected and silent.
#[tokio::test]
async fn stop_drains_a_connection_whose_peer_stays_idle() -> TestResult {
    let app = TestApp::default().route_response(ECHO_ROUTE, echo_handler())?;
    let listener = unused_listener()?;
    let server = WireframeServer::from_app(app)
        .workers(1)
        .bind_existing_listener(listener)?;
    let addr = server.local_addr().ok_or("server local address missing")?;
    let (ready_tx, ready_rx) = oneshot::channel();
    let shutdown = server.ready_signal(ready_tx).spawn().await?;
    wait_for_server_readiness(ready_rx).await?;

    let mut client = TcpStream::connect(addr).await?;
    client.write_all(&build_request(ECHO_ROUTE)?).await?;
    let codec = LengthDelimitedFrameCodec::new(MAX_FRAME_LENGTH);
    let mut frames = Framed::new(client, codec.decoder());
    let reply: Option<Result<Bytes, std::io::Error>> =
        timeout(STEP_TIMEOUT, frames.next())
            .await
            .map_err(|_| TestError::Msg("the handler never answered the request".into()))?;
    ensure(
        reply.transpose()?.is_some(),
        "the connection must answer before the peer falls idle",
    )?;

    // The peer now stays connected without sending another frame, so the
    // connection is parked in its inbound read when the stop arrives.
    shutdown.stop();

    timeout(STEP_TIMEOUT, shutdown.drained())
        .await
        .map_err(|_| {
            TestError::Msg("the drain waited for an idle peer that never sent anything".into())
        })??;
    Ok(())
}
