use bincode::config;
use bytes::BytesMut;
use tokio::io::{self, AsyncReadExt, AsyncWriteExt, duplex};
use wireframe::{
    app::{Envelope, Packet, WireframeApp},
    frame::{FrameMetadata, FrameProcessor, LengthPrefixedProcessor},
    serializer::Serializer,
};

pub trait TestSerializer:
    Serializer + FrameMetadata<Frame = Envelope> + Send + Sync + 'static
{
}
impl<T> TestSerializer for T where
    T: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync + 'static
{
}

const DEFAULT_CAPACITY: usize = 4096;

pub async fn drive_with_frame<S, C, E>(
    app: WireframeApp<S, C, E>,
    frame: Vec<u8>,
) -> io::Result<Vec<u8>>
where
    S: TestSerializer,
    C: Send + 'static,
    E: Packet,
{
    drive_with_frame_with_capacity(app, frame, DEFAULT_CAPACITY).await
}

pub async fn drive_with_frame_with_capacity<S, C, E>(
    app: WireframeApp<S, C, E>,
    frame: Vec<u8>,
    capacity: usize,
) -> io::Result<Vec<u8>>
where
    S: TestSerializer,
    C: Send + 'static,
    E: Packet,
{
    drive_with_frames_with_capacity(app, vec![frame], capacity).await
}

pub async fn drive_with_frames<S, C, E>(
    app: WireframeApp<S, C, E>,
    frames: Vec<Vec<u8>>,
) -> io::Result<Vec<u8>>
where
    S: TestSerializer,
    C: Send + 'static,
    E: Packet,
{
    drive_with_frames_with_capacity(app, frames, DEFAULT_CAPACITY).await
}

pub async fn drive_with_frames_with_capacity<S, C, E>(
    app: WireframeApp<S, C, E>,
    frames: Vec<Vec<u8>>,
    capacity: usize,
) -> io::Result<Vec<u8>>
where
    S: TestSerializer,
    C: Send + 'static,
    E: Packet,
{
    let (mut client, server) = duplex(capacity);
    let server_task = tokio::spawn(async move {
        app.handle_connection(server).await;
    });

    for frame in &frames {
        client.write_all(frame).await?;
    }
    client.shutdown().await?;

    let mut buf = Vec::new();
    client.read_to_end(&mut buf).await?;

    match server_task.await {
        Ok(_) => Ok(buf),
        Err(e) => Err(io::Error::new(
            io::ErrorKind::Other,
            format!("server task failed: {e}"),
        )),
    }
}

pub async fn drive_with_frame_mut<S, C, E>(
    app: &mut WireframeApp<S, C, E>,
    frame: Vec<u8>,
) -> io::Result<Vec<u8>>
where
    S: TestSerializer,
    C: Send + 'static,
    E: Packet,
{
    drive_with_frame_with_capacity_mut(app, frame, DEFAULT_CAPACITY).await
}

pub async fn drive_with_frame_with_capacity_mut<S, C, E>(
    app: &mut WireframeApp<S, C, E>,
    frame: Vec<u8>,
    capacity: usize,
) -> io::Result<Vec<u8>>
where
    S: TestSerializer,
    C: Send + 'static,
    E: Packet,
{
    drive_with_frames_with_capacity_mut(app, vec![frame], capacity).await
}

pub async fn drive_with_frames_mut<S, C, E>(
    app: &mut WireframeApp<S, C, E>,
    frames: Vec<Vec<u8>>,
) -> io::Result<Vec<u8>>
where
    S: TestSerializer,
    C: Send + 'static,
    E: Packet,
{
    drive_with_frames_with_capacity_mut(app, frames, DEFAULT_CAPACITY).await
}

pub async fn drive_with_frames_with_capacity_mut<S, C, E>(
    app: &mut WireframeApp<S, C, E>,
    frames: Vec<Vec<u8>>,
    capacity: usize,
) -> io::Result<Vec<u8>>
where
    S: TestSerializer,
    C: Send + 'static,
    E: Packet,
{
    let (mut client, server) = duplex(capacity);

    let server_fut = app.handle_connection(server);
    let client_fut = async {
        for frame in &frames {
            client.write_all(frame).await?;
        }
        client.shutdown().await?;

        let mut buf = Vec::new();
        client.read_to_end(&mut buf).await?;
        io::Result::Ok(buf)
    };

    let ((), buf) = tokio::join!(server_fut, client_fut);
    buf
}

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
    let bytes = bincode::encode_to_vec(msg, config::standard()).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("bincode encode failed: {e}"),
        )
    })?;
    let mut framed = BytesMut::new();
    LengthPrefixedProcessor::default().encode(&bytes, &mut framed)?;
    drive_with_frame(app, framed.to_vec()).await
}
