use tokio::io::{self, AsyncReadExt, AsyncWriteExt, duplex};
use wireframe::app::WireframeApp;

/// Feed a single frame into `app` and collect the response bytes.
///
/// # Errors
///
/// Propagates I/O errors from the in-memory connection.
///
/// # Panics
///
/// Panics if the spawned task running the application panics.
pub async fn run_app_with_frame(app: WireframeApp, frame: Vec<u8>) -> io::Result<Vec<u8>> {
    let (mut client, server) = duplex(1024);
    let server_task = tokio::spawn(async move {
        app.handle_connection(server).await;
    });

    client.write_all(&frame).await?;
    client.shutdown().await?;

    let mut buf = Vec::new();
    client.read_to_end(&mut buf).await?;

    server_task.await.unwrap();
    Ok(buf)
}
