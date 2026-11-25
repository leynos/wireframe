//! Guards the copied-byte accounting on the client send path.
//!
//! The client hands the serializer vector to the frame sink through
//! [`wireframe::benchmark_counters::bytes_from_vec`], the same seam the server
//! path uses, and records the serializer materialization at the client boundary
//! as well. Neither call had a test before this one: reverting the hand-off to
//! `Bytes::from` or deleting the recorder left every other suite green.
//!
//! The hand-off itself copies nothing while `bytes` shares the vector, so
//! [`wireframe::benchmark_counters::CopiedBytes::handoff`] cannot tell a seam
//! that ran from a seam that was bypassed. The seam-call count is what fails for
//! that reversion.
//!
//! This is its own test binary, and the only test in it, because the counters
//! are process-global: measuring them beside any other test would let that
//! test's sends land inside the window.
#![cfg(all(not(loom), feature = "benchmark-counters"))]
#![forbid(unsafe_code)]

use std::{error::Error, hint::black_box, io, net::SocketAddr};

use futures::StreamExt;
use tokio::{net::TcpListener, task::JoinHandle};
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use wireframe::{
    app::Envelope,
    benchmark_counters,
    client::WireframeClient,
    message::{DecodeWith, DeserializeContext, EncodeWith},
    serializer::{BincodeSerializer, MessageCompatibilitySerializer, Serializer},
};

/// Result alias for the test body.
type TestResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>;

/// Reports a mismatch as an error rather than a panic.
///
/// The test returns `Result`, and the workspace's Clippy configuration denies
/// panics in such functions, so a failed expectation must travel as an error.
fn check<T>(actual: &T, expected: &T, claim: &str) -> TestResult
where
    T: PartialEq + std::fmt::Debug + ?Sized,
{
    if actual == expected {
        Ok(())
    } else {
        Err(format!("{claim}: expected {expected:?}, got {actual:?}").into())
    }
}

/// Spare capacity the test serializer leaves behind its payload.
///
/// Bincode sizes its output exactly, so without this the vector would hand off
/// at `len == capacity` and the test would not exercise the spare-capacity
/// branch of the conversion at all.
const SPARE_CAPACITY_BYTES: usize = 256;

/// Bytes in the envelope the test sends.
///
/// Below the client codec's 1 KiB default frame limit: the measurement is about
/// the hand-off, not about frame sizing.
const PAYLOAD_BYTES: usize = 256;

/// Bincode serializer whose output keeps spare capacity for the hand-off.
///
/// The default [`BincodeSerializer`] returns an exact-capacity vector, so a
/// client built with it cannot show which branch of the hand-off ran. This
/// wrapper adds the spare capacity that a future `bytes` change could start
/// copying, which is exactly the case the counter guard exists to catch.
#[derive(Clone, Copy, Debug, Default)]
struct SpareCapacitySerializer;

impl MessageCompatibilitySerializer for SpareCapacitySerializer {}

impl Serializer for SpareCapacitySerializer {
    fn serialize<M>(&self, value: &M) -> Result<Vec<u8>, Box<dyn Error + Send + Sync>>
    where
        M: EncodeWith<Self>,
        Self: Sized,
    {
        let mut bytes = value.encode_with(self)?;
        bytes.reserve(SPARE_CAPACITY_BYTES);
        Ok(bytes)
    }

    fn deserialize<M>(&self, bytes: &[u8]) -> Result<(M, usize), Box<dyn Error + Send + Sync>>
    where
        M: DecodeWith<Self>,
        Self: Sized,
    {
        M::decode_with(self, bytes, &DeserializeContext::empty())
    }
}

/// Binds a loopback listener that drains every frame it receives.
///
/// The send path completes once the frame reaches the socket, so the peer only
/// has to accept and read; a reply would add a second seam to the measurement.
async fn spawn_draining_server() -> io::Result<(SocketAddr, JoinHandle<()>)> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;

    let task = tokio::spawn(async move {
        let Ok((stream, _)) = listener.accept().await else {
            return;
        };
        let mut framed = Framed::new(stream, LengthDelimitedCodec::new());
        while let Some(Ok(frame)) = framed.next().await {
            black_box(frame.len());
        }
    });

    Ok((addr, task))
}

#[tokio::test]
async fn client_send_hands_off_through_the_counter_seam() -> TestResult {
    let (addr, server) = spawn_draining_server().await?;
    let mut client = WireframeClient::builder()
        .serializer(SpareCapacitySerializer)
        .connect(addr)
        .await?;

    let envelope = Envelope::new(1, Some(7), vec![0x5a; PAYLOAD_BYTES]);
    // The encoder's own output length includes the envelope metadata, so it
    // cannot be derived from `PAYLOAD_BYTES` alone.
    let encoded_len = BincodeSerializer.serialize(&envelope)?.len() as u64;

    // Reset after connecting: the preamble exchange sends through the client
    // too, and its traffic belongs to connection setup, not this measurement.
    benchmark_counters::reset();
    client.send_envelope(envelope).await?;
    let copied = benchmark_counters::snapshot();

    check(
        &copied.serialized,
        &encoded_len,
        "the client boundary must record the serializer's materialization",
    )?;
    check(
        &copied.handoff,
        &0,
        "a spare-capacity vector must hand off without copying its payload",
    )?;
    check(
        &copied.framed,
        &0,
        "the client hands the frame to the sink; it does not frame into a transport buffer",
    )?;
    check(
        &benchmark_counters::handoff_seam_calls(),
        &1,
        "the send path must hand off through `bytes_from_vec`; reverting to `Bytes::from` leaves \
         this at zero",
    )?;

    drop(client);
    server.await?;
    Ok(())
}
