//! Test-only helpers for shared test utilities.
#![cfg(any(test, feature = "test-support"))]

use std::io;

use bytes::{Buf, BufMut, BytesMut};

use crate::message_assembler::{
    ContinuationFrameHeader,
    FirstFrameHeader,
    FrameHeader,
    FrameSequence,
    MessageAssembler,
    MessageKey,
    ParsedFrameHeader,
};

pub mod bad_codec;

pub mod connection_startup;

pub mod frame_codec;
// `pool_client` drives a real TCP client, and both `crate::client` and
// `tokio::net` are compiled out under `cfg(loom)`. The feature alone is not
// enough: the crate's self dev-dependency enables `pool` in every test build
// through feature unification, so the Loom lane selects it without asking.
#[cfg(all(feature = "pool", not(loom)))]
pub mod pool_builders;
#[cfg(all(feature = "pool", not(loom)))]
pub mod pool_client;

pub mod teardown;

pub use bad_codec::{BadCodec, BadDecoder, BadEncoder, BadFrame};
pub use connection_startup::{
    ConnectionStartupCounts,
    ConnectionStartupInstrumentation,
    StartupFixturePlan,
    StartupTestApp,
    TransformCountingMiddleware,
    build_counted_app,
    counted_app_factory,
    noop_handler,
};
pub use frame_codec::{CloneCountingCodec, TestAdapter, TestCodec, TestFrame};
#[cfg(all(feature = "pool", not(loom)))]
pub use pool_builders::{
    LifecycleCounters,
    LifecycleGeneration,
    LifecycleTestClientPool,
    build_lifecycle_pool,
    build_pooled_client,
    build_preamble_pool,
};
#[cfg(all(feature = "pool", not(loom)))]
pub use pool_client::{
    ClientHello,
    Ping,
    Pong,
    PoolServerBehavior,
    PoolTestServer,
    TestClientPool,
    acquire_and_record,
};
pub use teardown::{UnitFuture, counting_teardown};

/// Test-friendly message assembler implementation that shares parsing logic.
#[derive(Clone, Copy, Debug, Default)]
pub struct TestAssembler;

impl MessageAssembler for TestAssembler {
    fn parse_frame_header(&self, payload: &[u8]) -> Result<ParsedFrameHeader, io::Error> {
        parse_frame_header(payload)
    }
}

#[derive(Clone, Copy, Debug)]
/// Bit flags encoded in the test protocol's frame header.
struct FrameFlags(u8);

impl FrameFlags {
    /// Whether this frame terminates the logical message.
    const fn is_last(self) -> bool { self.0 & 0b1 == 0b1 }

    /// Whether the header carries its optional length/sequence value.
    const fn has_optional_field(self) -> bool { self.0 & 0b10 == 0b10 }
}

/// Parse a protocol-specific frame header for tests.
///
/// # Errors
///
/// Returns an error if the payload is too short or contains an invalid header.
pub fn parse_frame_header(payload: &[u8]) -> Result<ParsedFrameHeader, io::Error> {
    let mut buf = payload;
    let initial = buf.remaining();

    let kind = take_u8(&mut buf)?;
    let flags = FrameFlags(take_u8(&mut buf)?);
    let message_key = MessageKey::from(take_u64(&mut buf)?);

    let header = match kind {
        0x01 => parse_first_frame_header(&mut buf, flags, message_key)?,
        0x02 => parse_continuation_frame_header(&mut buf, flags, message_key)?,
        _ => return Err(invalid_data("unknown header kind")),
    };

    let header_len = initial - buf.remaining();
    Ok(ParsedFrameHeader::new(header, header_len))
}

/// Parse the first-frame fields after the common header prefix.
fn parse_first_frame_header(
    buf: &mut &[u8],
    flags: FrameFlags,
    message_key: MessageKey,
) -> Result<FrameHeader, io::Error> {
    let metadata_len = usize::from(take_u16(buf)?);
    let body_len = take_usize_u32(buf, "body length too large")?;
    let total_body_len = take_optional_usize_u32(buf, flags, "total length too large")?;

    Ok(FrameHeader::First(FirstFrameHeader {
        message_key,
        metadata_len,
        body_len,
        total_body_len,
        is_last: flags.is_last(),
    }))
}

/// Parse continuation-frame fields after the common header prefix.
fn parse_continuation_frame_header(
    buf: &mut &[u8],
    flags: FrameFlags,
    message_key: MessageKey,
) -> Result<FrameHeader, io::Error> {
    let body_len = take_usize_u32(buf, "body length too large")?;
    let sequence = take_optional_sequence(buf, flags)?;

    Ok(FrameHeader::Continuation(ContinuationFrameHeader {
        message_key,
        sequence,
        body_len,
        is_last: flags.is_last(),
    }))
}

/// Read a u32 length and convert it to the platform's `usize`.
fn take_usize_u32(buf: &mut &[u8], message: &'static str) -> Result<usize, io::Error> {
    usize::try_from(take_u32(buf)?).map_err(|_| invalid_data(message))
}

/// Read an optional u32 length when the header flag advertises one.
fn take_optional_usize_u32(
    buf: &mut &[u8],
    flags: FrameFlags,
    message: &'static str,
) -> Result<Option<usize>, io::Error> {
    if !flags.has_optional_field() {
        return Ok(None);
    }

    take_usize_u32(buf, message).map(Some)
}

/// Read an optional continuation sequence when its flag is present.
fn take_optional_sequence(
    buf: &mut &[u8],
    flags: FrameFlags,
) -> Result<Option<FrameSequence>, io::Error> {
    if !flags.has_optional_field() {
        return Ok(None);
    }

    Ok(Some(FrameSequence::from(take_u32(buf)?)))
}

/// Read one byte after checking that the header buffer is long enough.
fn take_u8(buf: &mut &[u8]) -> Result<u8, io::Error> {
    ensure_remaining(buf, 1)?;
    Ok(buf.get_u8())
}

/// Read a big-endian u16 after checking header bounds.
fn take_u16(buf: &mut &[u8]) -> Result<u16, io::Error> {
    ensure_remaining(buf, 2)?;
    Ok(buf.get_u16())
}

/// Read a big-endian u32 after checking header bounds.
fn take_u32(buf: &mut &[u8]) -> Result<u32, io::Error> {
    ensure_remaining(buf, 4)?;
    Ok(buf.get_u32())
}

/// Read a big-endian u64 after checking header bounds.
fn take_u64(buf: &mut &[u8]) -> Result<u64, io::Error> {
    ensure_remaining(buf, 8)?;
    Ok(buf.get_u64())
}

/// Reject a header read that would consume beyond the available bytes.
fn ensure_remaining(buf: &mut &[u8], needed: usize) -> Result<(), io::Error> {
    if buf.remaining() < needed {
        return Err(invalid_data("header too short"));
    }
    Ok(())
}

/// Build the consistent invalid-data error returned by header parsing.
fn invalid_data(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

/// Contents of a first frame the test protocol should encode.
///
/// The payload fields travel together because the encoded header and payload
/// must agree: `metadata_len` and `body_len` are derived from the slices here,
/// so a caller cannot describe a frame whose header contradicts its payload.
#[derive(Clone, Copy, Debug)]
pub struct FirstFrameSpec<'a> {
    /// Message key the frame belongs to.
    pub key: MessageKey,
    /// Metadata bytes placed ahead of the body, possibly empty.
    pub metadata: &'a [u8],
    /// Body bytes following the metadata.
    pub body: &'a [u8],
    /// Whether this frame completes its message.
    pub is_last: bool,
    /// Declared total body length, when the protocol supplies one.
    pub total: Option<u32>,
}

/// Build a first-frame payload for the test protocol with no metadata.
///
/// # Errors
///
/// Returns an error if the body length exceeds `u32::MAX`.
pub fn first_frame_payload(
    key: MessageKey,
    body: &[u8],
    is_last: bool,
    total: Option<u32>,
) -> Result<Vec<u8>, io::Error> {
    first_frame_payload_with_metadata(FirstFrameSpec {
        key,
        metadata: &[],
        body,
        is_last,
        total,
    })
}

/// Build a first-frame payload carrying metadata ahead of its body.
///
/// The encoded payload places the metadata bytes before the body bytes, which
/// is the split the inbound adapter performs using `metadata_len`.
///
/// # Errors
///
/// Returns an error if the metadata length exceeds `u16::MAX` or the body
/// length exceeds `u32::MAX`.
pub fn first_frame_payload_with_metadata(spec: FirstFrameSpec<'_>) -> Result<Vec<u8>, io::Error> {
    let FirstFrameSpec {
        key,
        metadata,
        body,
        is_last,
        total,
    } = spec;

    let mut payload = BytesMut::new();
    payload.put_u8(0x01);
    let mut flags = 0u8;
    if is_last {
        flags |= 0b1;
    }
    if total.is_some() {
        flags |= 0b10;
    }
    payload.put_u8(flags);
    payload.put_u64(u64::from(key));
    let metadata_len =
        u16::try_from(metadata.len()).map_err(|_| invalid_data("metadata length too large"))?;
    payload.put_u16(metadata_len);
    let body_len = u32::try_from(body.len()).map_err(|_| invalid_data("body length too large"))?;
    payload.put_u32(body_len);
    if let Some(total_body_len) = total {
        payload.put_u32(total_body_len);
    }
    payload.extend_from_slice(metadata);
    payload.extend_from_slice(body);
    Ok(payload.to_vec())
}

/// Build a continuation-frame payload for the test protocol.
///
/// # Errors
///
/// Returns an error if the body length exceeds `u32::MAX`.
pub fn continuation_frame_payload(
    key: MessageKey,
    sequence: FrameSequence,
    body: &[u8],
    is_last: bool,
) -> Result<Vec<u8>, io::Error> {
    let mut payload = BytesMut::new();
    payload.put_u8(0x02);
    let mut flags = 0b10;
    if is_last {
        flags |= 0b1;
    }
    payload.put_u8(flags);
    payload.put_u64(u64::from(key));
    let body_len = u32::try_from(body.len()).map_err(|_| invalid_data("body length too large"))?;
    payload.put_u32(body_len);
    payload.put_u32(u32::from(sequence));
    payload.extend_from_slice(body);
    Ok(payload.to_vec())
}
