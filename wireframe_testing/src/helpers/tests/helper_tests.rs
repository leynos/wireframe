//! Verifies helper utilities such as `run_app`, `drive_with_payloads`,
//! `decode_frames`, and `MAX_CAPACITY` handling.

use std::{io, sync::Arc};

use futures::future::BoxFuture;
use tokio::io::DuplexStream;
use wireframe::{
    app::{Envelope, WireframeApp},
    prelude::Serializer,
    serializer::BincodeSerializer,
};

use super::super::drive::drive_internal;
use crate::helpers::{MAX_CAPACITY, decode_frames, drive_with_payloads, run_app};

/// Convert synchronous server-factory panics into the documented I/O error.
#[tokio::test]
async fn drive_internal_converts_synchronous_server_panics_to_io_errors() {
    let result = drive_internal(
        |_: DuplexStream| -> std::future::Ready<io::Result<()>> {
            panic!("synchronous server factory panic")
        },
        Vec::new(),
        64,
    )
    .await;

    let error = result.expect_err("synchronous server panic should become an I/O error");
    assert_eq!(error.kind(), io::ErrorKind::Other);
    assert!(
        error.to_string().starts_with("server task failed"),
        "unexpected panic conversion: {error}"
    );
}

#[tokio::test]
async fn run_app_rejects_zero_capacity() {
    let app: WireframeApp<BincodeSerializer, (), Envelope> =
        WireframeApp::new().expect("failed to create app");
    let err = run_app(app, vec![], Some(0))
        .await
        .expect_err("capacity of zero should error");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
}

#[tokio::test]
async fn run_app_rejects_excess_capacity() {
    let app: WireframeApp<BincodeSerializer, (), Envelope> =
        WireframeApp::new().expect("failed to create app");
    let err = run_app(app, vec![], Some(MAX_CAPACITY + 1))
        .await
        .expect_err("capacity beyond max should error");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
}

#[tokio::test]
async fn drive_with_payloads_wraps_frames() -> io::Result<()> {
    let app: WireframeApp<BincodeSerializer, (), Envelope> =
        WireframeApp::new().map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("failed to create app: {error}"),
            )
        })?;
    let app = app
        .route(
            1,
            Arc::new(|_: &Envelope| -> BoxFuture<'static, ()> { Box::pin(async {}) }),
        )
        .map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("route registration should succeed: {error}"),
            )
        })?;
    let serializer = BincodeSerializer;
    let payload = vec![1_u8, 2, 3];
    let env = Envelope::new(1, Some(7), payload.clone());
    let encoded = serializer.serialize(&env).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("failed to serialize envelope: {error}"),
        )
    })?;

    let out = drive_with_payloads(app, vec![encoded]).await?;
    let frames = decode_frames(out)?;
    if frames.len() != 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("expected a single response frame, got {}", frames.len()),
        ));
    }
    let first = frames.first().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "expected a single response frame",
        )
    })?;
    let (decoded, _) = serializer.deserialize::<Envelope>(first).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("failed to deserialise envelope: {error}"),
        )
    })?;
    if decoded.payload_bytes() != payload.as_slice() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "response payload did not preserve the request payload",
        ));
    }
    Ok(())
}
