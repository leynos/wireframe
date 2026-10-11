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

/// A reassembled response payload plus the number of frames that carried it.
///
/// The frame count is kept separately so a test can distinguish a genuinely
/// unfragmented response from a fragmented one, which
/// [`read_reassembled_response`] alone cannot show.
pub struct FramedResponse {
    /// Reassembled response payload.
    pub payload: Vec<u8>,
    /// Number of transport frames observed for this response.
    pub frame_count: usize,
}

/// Read a response, preserving the frame boundaries that carried it.
///
/// Returns once the payload reassembles, together with the number of frames
/// consumed. A single frame means the response was not fragmented.
///
/// # Errors
///
/// Returns an error if reading, decoding the envelope, or reassembly fails,
/// or if the stream ends before reassembly completes.
pub async fn read_framed_response(
    client: &mut Framed<tokio::io::DuplexStream, LengthDelimitedCodec>,
    cfg: &FragmentationConfig,
) -> TestResult<FramedResponse> {
    let serializer = BincodeSerializer;
    let mut reassembler = Reassembler::new(cfg.max_message_size, cfg.reassembly_timeout);
    let mut frame_count = 0usize;

    while let Some(frame) = client.next().await {
        let bytes = frame?;
        frame_count += 1;
        let (env, _) = serializer.deserialize::<Envelope>(&bytes)?;
        let payload = env.into_parts().into_payload();
        match decode_fragment_payload(&payload)? {
            Some((header, fragment)) => {
                if let Some(message) = reassembler.push(header, fragment)? {
                    return Ok(FramedResponse {
                        payload: message.into_payload(),
                        frame_count,
                    });
                }
            }
            None => {
                return Ok(FramedResponse {
                    payload,
                    frame_count,
                });
            }
        }
    }

    Err(TestError::Setup(
        "response stream ended before reassembly completed",
    ))
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
    Ok(read_framed_response(client, cfg).await?.payload)
}

/// Read the response with a 1-second timeout, preserving frame boundaries.
///
/// # Errors
///
/// Returns an error if the read times out or reassembly fails.
pub async fn read_response_payload(
    client: &mut Framed<tokio::io::DuplexStream, LengthDelimitedCodec>,
    config: &FragmentationConfig,
) -> TestResult<FramedResponse> {
    let response = timeout(
        TokioDuration::from_secs(1),
        read_framed_response(client, config),
    )
    .await??;
    Ok(response)
}
