//! Framed transport helpers for fragment integration tests.

use futures::{SinkExt, StreamExt};
use tokio::time::{Duration as TokioDuration, timeout};
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use wireframe::{
    app::{Envelope, Packet},
    fragment::{FragmentationConfig, Reassembler, decode_fragment_payload},
    serializer::{BincodeSerializer, Serializer},
};

use super::{TestError, TestResult};

/// Send a slice of envelopes over a framed client connection.
///
/// # Errors
///
/// Returns an error if encoding the envelope or sending fails.
pub async fn send_envelopes(
    client: &mut Framed<tokio::io::DuplexStream, LengthDelimitedCodec>,
    envelopes: &[Envelope],
) -> TestResult {
    let serializer = BincodeSerializer;
    for env in envelopes {
        let bytes = serializer.serialize(env)?;
        client.send(bytes.into()).await?;
    }
    Ok(())
}

/// Read and reassemble a fragmented response from a client connection.
///
/// # Errors
///
/// Returns an error if reading, decoding the envelope, or reassembly fails,
/// or if the stream ends before reassembly completes.
pub async fn read_reassembled_response(
    client: &mut Framed<tokio::io::DuplexStream, LengthDelimitedCodec>,
    cfg: &FragmentationConfig,
) -> TestResult<Vec<u8>> {
    let serializer = BincodeSerializer;
    let mut reassembler = Reassembler::new(cfg.max_message_size, cfg.reassembly_timeout);

    while let Some(frame) = client.next().await {
        let bytes = frame?;
        let (env, _) = serializer.deserialize::<Envelope>(&bytes)?;
        let payload = env.into_parts().into_payload();
        match decode_fragment_payload(&payload)? {
            Some((header, fragment)) => {
                if let Some(message) = reassembler.push(header, fragment)? {
                    return Ok(message.into_payload());
                }
            }
            None => return Ok(payload),
        }
    }

    Err(TestError::Setup(
        "response stream ended before reassembly completed",
    ))
}

/// Read and return the response payload with a 1-second timeout.
///
/// # Errors
///
/// Returns an error if the read times out or reassembly fails.
pub async fn read_response_payload(
    client: &mut Framed<tokio::io::DuplexStream, LengthDelimitedCodec>,
    config: &FragmentationConfig,
) -> TestResult<Vec<u8>> {
    let response = timeout(
        TokioDuration::from_secs(1),
        read_reassembled_response(client, config),
    )
    .await??;
    Ok(response)
}
