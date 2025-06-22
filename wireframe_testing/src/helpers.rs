use bincode::Encode;
use bytes::BytesMut;
use tokio::io::{self, AsyncReadExt, AsyncWriteExt, DuplexStream, duplex};
use wireframe::{
    app::{Packet, WireframeApp},
    frame::FrameProcessor,
    serializer::Serializer,
};

const DEFAULT_CAPACITY: usize = 4096;

/// Feed a single frame into `app` using an in-memory duplex stream.
pub async fn drive_with_frame<S, C, E>(
    app: WireframeApp<S, C, E>,
    frame: Vec<u8>,
) -> io::Result<Vec<u8>>
where
    S: Serializer + Send + Sync + 'static,
    C: Send + 'static,
    E: Packet,
{
    drive_with_frame_with_capacity(app, frame, DEFAULT_CAPACITY).await
}

/// Drive `app` with multiple frames, returning all bytes written by the app.
pub async fn drive_with_frames<S, C, E>(
    app: WireframeApp<S, C, E>,
    frames: Vec<Vec<u8>>,
) -> io::Result<Vec<u8>>
where
    S: Serializer + Send + Sync + 'static,
    C: Send + 'static,
    E: Packet,
{
    drive_with_frames_with_capacity(app, frames, DEFAULT_CAPACITY).await
}

/// Feed `app` a single frame with a custom duplex buffer capacity.
pub async fn drive_with_frame_with_capacity<S, C, E>(
    app: WireframeApp<S, C, E>,
    frame: Vec<u8>,
    capacity: usize,
) -> io::Result<Vec<u8>>
where
    S: Serializer + Send + Sync + 'static,
    C: Send + 'static,
    E: Packet,
{
    drive_with_frames_with_capacity(app, vec![frame], capacity).await
}

/// Drive `app` with multiple frames using a duplex buffer of `capacity` bytes.
pub async fn drive_with_frames_with_capacity<S, C, E>(
    app: WireframeApp<S, C, E>,
    frames: Vec<Vec<u8>>,
    capacity: usize,
) -> io::Result<Vec<u8>>
where
    S: Serializer + Send + Sync + 'static,
    C: Send + 'static,
    E: Packet,
{
    let (mut client, server) = duplex(capacity);
    let server_task = tokio::spawn(async move {
        app.handle_connection(server).await;
    });

    send_frames(&mut client, &frames).await?;
    client.shutdown().await?;

    let mut buf = Vec::new();
    client.read_to_end(&mut buf).await?;

    server_task.await.expect("app task panicked");
    Ok(buf)
}

/// Borrow `app` mutably and feed it a single frame.
pub async fn drive_with_frame_mut<S, C, E>(
    app: &mut WireframeApp<S, C, E>,
    frame: Vec<u8>,
) -> io::Result<Vec<u8>>
where
    S: Serializer + Send + Sync,
    C: Send,
    E: Packet,
{
    drive_with_frames_mut(app, vec![frame]).await
}

/// Borrow `app` mutably and feed it multiple frames.
pub async fn drive_with_frames_mut<S, C, E>(
    app: &mut WireframeApp<S, C, E>,
    frames: Vec<Vec<u8>>,
) -> io::Result<Vec<u8>>
where
    S: Serializer + Send + Sync,
    C: Send,
    E: Packet,
{
    let (mut client, server) = duplex(DEFAULT_CAPACITY);

    send_frames(&mut client, &frames).await?;
    client.shutdown().await?;

    app.handle_connection(server).await;

    let mut buf = Vec::new();
    client.read_to_end(&mut buf).await?;

    Ok(buf)
}

/// Encode `msg` using `bincode` and drive the app with the resulting frame.
pub async fn drive_with_bincode<S, C, E, M>(
    app: WireframeApp<S, C, E>,
    msg: M,
) -> io::Result<Vec<u8>>
where
    S: Serializer + Send + Sync + 'static,
    C: Send + 'static,
    E: Packet,
    M: Encode,
{
    let bytes = bincode::encode_to_vec(msg, bincode::config::standard())
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    let mut framed = BytesMut::with_capacity(4 + bytes.len());
    wireframe::frame::LengthPrefixedProcessor::default()
        .encode(&bytes, &mut framed)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    drive_with_frame(app, framed.to_vec()).await
}

async fn send_frames(stream: &mut DuplexStream, frames: &[Vec<u8>]) -> io::Result<()> {
    for frame in frames {
        stream.write_all(frame).await?;
    }
    Ok(())
}
