//! Shared transport helpers for unified codec tests.
//!
//! Provides common send/receive helpers used by both integration and
//! behavioural unified codec tests.

use std::time::Duration;

use futures::{SinkExt, StreamExt};
use tokio::time::timeout;
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use wireframe::{Serializer, app::Envelope, serializer::BincodeSerializer};

/// Shared result type for helper functions in this module.
pub type SharedTestResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Serialize and send one envelope to a framed test client.
///
/// # Errors
/// Returns an error if serialization or sending fails.
pub async fn send_one(
    client: &mut Framed<tokio::io::DuplexStream, LengthDelimitedCodec>,
    envelope: &Envelope,
) -> SharedTestResult {
    let serializer = BincodeSerializer;
    let bytes = serializer.serialize(envelope)?;
    client.send(bytes.into()).await?;
    Ok(())
}

/// Receive and deserialize one envelope from a framed test client.
///
/// # Errors
/// Returns an error if reading, timeout, or deserialization fails.
pub async fn recv_one(
    client: &mut Framed<tokio::io::DuplexStream, LengthDelimitedCodec>,
) -> SharedTestResult<Envelope> {
    let serializer = BincodeSerializer;
    let frame = timeout(Duration::from_secs(2), client.next())
        .await?
        .ok_or("response frame missing")??;
    let (env, _) = serializer.deserialize::<Envelope>(&frame)?;
    Ok(env)
}
