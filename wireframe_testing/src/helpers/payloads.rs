//! Payload-oriented in-memory driving helpers.

use std::io;

use bincode::config;
use bytes::BytesMut;
use tokio_util::codec::{Encoder, LengthDelimitedCodec};
use wireframe::{
    app::{Packet, WireframeApp},
    frame::LengthFormat,
};

use super::{DEFAULT_CAPACITY, TestSerializer, drive, new_test_codec};

/// Encode payloads as length-delimited frames and drive `app`.
///
/// This helper wraps each payload using the default length-delimited framing
/// format before sending it to the application.
///
/// ```rust
/// # use wireframe_testing::drive_with_payloads;
/// # use wireframe::app::WireframeApp;
/// # async fn demo() -> std::io::Result<()> {
/// let app = WireframeApp::new().expect("failed to initialize app");
/// let out = drive_with_payloads(app, vec![vec![1], vec![2]]).await?;
/// # let _ = out;
/// # Ok(())
/// # }
/// ```
pub async fn drive_with_payloads<S, C, E>(
    app: WireframeApp<S, C, E>,
    payloads: Vec<Vec<u8>>,
) -> io::Result<Vec<u8>>
where
    S: TestSerializer,
    C: Send + 'static,
    E: Packet,
{
    drive_with_payloads_with_capacity(app, payloads, DEFAULT_CAPACITY).await
}

/// Encode payloads as length-delimited frames and drive a mutable `app`.
///
/// ```rust
/// # use wireframe_testing::drive_with_payloads_mut;
/// # use wireframe::app::WireframeApp;
/// # async fn demo() -> std::io::Result<()> {
/// let mut app = WireframeApp::new().expect("failed to initialize app");
/// let out = drive_with_payloads_mut(&mut app, vec![vec![1], vec![2]]).await?;
/// # let _ = out;
/// # Ok(())
/// # }
/// ```
pub async fn drive_with_payloads_mut<S, C, E>(
    app: &mut WireframeApp<S, C, E>,
    payloads: Vec<Vec<u8>>,
) -> io::Result<Vec<u8>>
where
    S: TestSerializer,
    C: Send + 'static,
    E: Packet,
{
    drive_with_payloads_with_capacity_mut(app, payloads, DEFAULT_CAPACITY).await
}

fn encode_payloads(
    payloads: Vec<Vec<u8>>,
    mut codec: LengthDelimitedCodec,
) -> io::Result<Vec<Vec<u8>>> {
    payloads
        .into_iter()
        .map(|payload| {
            let header_len = LengthFormat::default().bytes();
            let mut buf = BytesMut::with_capacity(payload.len() + header_len);
            codec.encode(payload.into(), &mut buf).map_err(|err| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("frame encode failed: {err}"),
                )
            })?;
            Ok(buf.to_vec())
        })
        .collect()
}

async fn drive_with_payloads_with_capacity<S, C, E>(
    app: WireframeApp<S, C, E>,
    payloads: Vec<Vec<u8>>,
    capacity: usize,
) -> io::Result<Vec<u8>>
where
    S: TestSerializer,
    C: Send + 'static,
    E: Packet,
{
    let codec = new_test_codec(DEFAULT_CAPACITY);
    let frames = encode_payloads(payloads, codec)?;
    drive::drive_with_frames_with_capacity(app, frames, capacity).await
}

async fn drive_with_payloads_with_capacity_mut<S, C, E>(
    app: &mut WireframeApp<S, C, E>,
    payloads: Vec<Vec<u8>>,
    capacity: usize,
) -> io::Result<Vec<u8>>
where
    S: TestSerializer,
    C: Send + 'static,
    E: Packet,
{
    let codec = new_test_codec(DEFAULT_CAPACITY);
    let frames = encode_payloads(payloads, codec)?;
    drive::drive_with_frames_with_capacity_mut(app, frames, capacity).await
}

/// Encode `msg` using bincode, frame it and drive `app`.
///
/// ```rust
/// # use wireframe_testing::drive_with_bincode;
/// # use wireframe::app::WireframeApp;
/// #[derive(bincode::Encode)]
/// struct Ping(u8);
/// # async fn demo() -> std::io::Result<()> {
/// let app = WireframeApp::new().expect("failed to initialize app");
/// let bytes = drive_with_bincode(app, Ping(1)).await?;
/// # Ok(())
/// # }
/// ```
pub async fn drive_with_bincode<M, S, C, E>(
    app: WireframeApp<S, C, E>,
    msg: M,
) -> io::Result<Vec<u8>>
where
    M: bincode::Encode,
    S: TestSerializer,
    C: Send + 'static,
    E: Packet,
{
    let bytes = bincode::encode_to_vec(msg, config::standard()).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("bincode encode failed: {error}"),
        )
    })?;
    let mut codec = new_test_codec(DEFAULT_CAPACITY);
    let mut prefix_probe = BytesMut::new();
    codec.encode(Vec::<u8>::new().into(), &mut prefix_probe)?;
    let prefix_len = prefix_probe.len();
    let mut framed = BytesMut::with_capacity(bytes.len() + prefix_len);
    codec.encode(bytes.into(), &mut framed)?;
    drive::drive_with_frame(app, framed.to_vec()).await
}
