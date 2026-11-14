//! Regressions for preamble allocation limits and the read-buffer growth guard.

use std::{
    io::{self, Cursor},
    pin::Pin,
    task::{Context, Poll},
};

use rstest::rstest;
use tokio::io::{AsyncRead, AsyncReadExt, ReadBuf};
use wireframe_testing::TestResult;

use super::{MAX_PREAMBLE_LEN, PREAMBLE_TOO_LONG_ERROR, read_more, read_preamble};

/// Fixed I/O message the terminal reader reports, letting a test prove that
/// `read_more` preserves the original error alongside the shortfall count.
const INJECTED_READ_ERROR: &str = "injected preamble read failure";

/// Configured preamble type containing an owned, peer-sized container.
#[derive(Debug, PartialEq, bincode::Encode, bincode::Decode)]
struct ContainerPreamble(Vec<u8>);

/// Terminal stream that ends the read or fails with a chosen I/O kind.
///
/// The stub never fills the buffer, so a [`Cursor`] chained ahead of it
/// delivers a fixed prefix first and `read_more` then observes the requested
/// terminal path.
#[derive(Debug)]
struct TerminalReader {
    /// Fault reported instead of end-of-stream when present.
    fault: Option<io::ErrorKind>,
}

impl AsyncRead for TerminalReader {
    fn poll_read(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        _buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        self.fault.map_or(Poll::Ready(Ok(())), |kind| {
            Poll::Ready(Err(io::Error::new(kind, INJECTED_READ_ERROR)))
        })
    }
}

#[rstest]
#[case::above_preamble_ceiling(MAX_PREAMBLE_LEN as u64 + 1)]
#[case::maximum_native_length(usize::MAX as u64)]
#[tokio::test]
async fn oversized_preamble_container_fails_before_reading_its_body(#[case] declared: u64) {
    // Fixed-width big-endian encoding allows the entire attack input to fit
    // in eight bytes, without constructing the declared container.
    let prefix = crate::byte_order::write_network_u64(declared);
    let mut reader = Cursor::new(prefix);
    let error = read_preamble::<_, ContainerPreamble>(&mut reader)
        .await
        .expect_err("declared allocation must exceed the preamble budget");
    assert!(
        matches!(error, bincode::error::DecodeError::LimitExceeded),
        "{error:?}"
    );
    assert_eq!(reader.position(), 8, "reader.position() should equal 8");
    assert!(
        reader.position() <= MAX_PREAMBLE_LEN as u64,
        "assertion failed: reader.position() <= MAX_PREAMBLE_LEN as u64"
    );
}

#[tokio::test]
async fn preamble_read_buffer_never_grows_past_ceiling() {
    let mut buffer = vec![0; MAX_PREAMBLE_LEN];
    let mut reader = Cursor::new([1]);
    let error = read_more(&mut reader, &mut buffer, 1)
        .await
        .expect_err("growth past the preamble cap must fail");
    assert!(
        matches!(
            error,
            bincode::error::DecodeError::Other(PREAMBLE_TOO_LONG_ERROR)
        ),
        "assertion failed: matches!( error, \
         bincode::error::DecodeError::Other(PREAMBLE_TOO_LONG_ERROR) )"
    );
    assert_eq!(
        buffer.len(),
        MAX_PREAMBLE_LEN,
        "buffer.len() should equal MAX_PREAMBLE_LEN"
    );
    assert_eq!(reader.position(), 0, "reader.position() should equal 0");
}

#[tokio::test]
async fn preamble_read_buffer_accepts_exactly_the_ceiling() {
    let source = vec![0x5a_u8; MAX_PREAMBLE_LEN];
    let mut buffer = Vec::new();
    let mut reader = Cursor::new(source.as_slice());
    read_more(&mut reader, &mut buffer, MAX_PREAMBLE_LEN)
        .await
        .expect("a buffer that lands exactly on the ceiling must be accepted");
    assert_eq!(
        buffer.len(),
        MAX_PREAMBLE_LEN,
        "buffer.len() should equal MAX_PREAMBLE_LEN"
    );
    assert_eq!(buffer, source, "buffer should equal source");
    assert_eq!(
        reader.position(),
        MAX_PREAMBLE_LEN as u64,
        "reader.position() should equal MAX_PREAMBLE_LEN as u64"
    );
}

#[rstest]
#[case::eof_after_partial_read(None, io::ErrorKind::UnexpectedEof)]
#[case::reader_error_after_partial_read(
    Some(io::ErrorKind::ConnectionReset),
    io::ErrorKind::ConnectionReset
)]
#[tokio::test]
async fn partial_preamble_read_reports_missing_bytes(
    #[case] fault: Option<io::ErrorKind>,
    #[case] expected_kind: io::ErrorKind,
) -> TestResult {
    // Seven bytes requested, two delivered by the prefix reader, so the
    // shortfall arithmetic must report five.
    let mut buffer = Vec::new();
    let mut reader = Cursor::new([0xaa_u8, 0xbb]).chain(TerminalReader { fault });
    let error = read_more(&mut reader, &mut buffer, 7)
        .await
        .expect_err("a terminal reader must leave the requested bytes unread");
    let (inner, additional) = match error {
        bincode::error::DecodeError::Io { inner, additional } => (inner, additional),
        other => {
            return Err(
                format!("expected DecodeError::Io after a partial read, got {other:?}").into(),
            );
        }
    };
    if additional != 5 {
        return Err(
            format!("expected 5 missing bytes after a 2-byte prefix, got {additional}").into(),
        );
    }
    if inner.kind() != expected_kind {
        return Err(format!(
            "expected terminal I/O kind {expected_kind:?}, got {:?}",
            inner.kind()
        )
        .into());
    }
    if fault.is_some() && inner.to_string() != INJECTED_READ_ERROR {
        return Err(format!("the injected I/O error must be preserved, got {inner}").into());
    }
    if buffer.get(..2) != Some([0xaa_u8, 0xbb].as_slice()) {
        return Err(format!("the two-byte prefix must be retained, got {buffer:?}").into());
    }
    Ok(())
}

#[rstest]
#[case::empty(0)]
#[case::at_accounted_limit(MAX_PREAMBLE_LEN - std::mem::size_of::<u64>())]
#[tokio::test]
async fn bounded_preamble_preserves_valid_containers_and_following_bytes(#[case] length: usize) {
    let value = ContainerPreamble(vec![7; length]);
    let mut bytes = bincode::encode_to_vec(
        &value,
        bincode::config::standard()
            .with_big_endian()
            .with_fixed_int_encoding(),
    )
    .expect("encode valid preamble");
    let encoded_len = bytes.len();
    bytes.extend_from_slice(&[9, 10]);
    let mut reader = Cursor::new(bytes);
    let (decoded, leftover) = read_preamble::<_, ContainerPreamble>(&mut reader)
        .await
        .expect("preamble within the budget must decode");
    assert_eq!(decoded, value, "decoded should equal value");
    assert_eq!(
        leftover,
        [] as [u8; 0],
        "leftover should equal [] as [u8; 0]"
    );
    assert_eq!(
        reader.position(),
        encoded_len as u64,
        "reader.position() should equal encoded_len as u64"
    );
    assert!(
        encoded_len <= MAX_PREAMBLE_LEN,
        "assertion failed: encoded_len <= MAX_PREAMBLE_LEN"
    );
    assert_eq!(
        reader.get_ref().get(encoded_len..),
        Some([9, 10].as_slice()),
        "reader.get_ref().get(encoded_len..) should equal Some([9, 10].as_slice())"
    );
}

#[tokio::test]
async fn preamble_returns_read_ahead_bytes_as_leftovers() {
    let mut reader = Cursor::new([9, 10]);
    let ((), leftover) = read_preamble::<_, ()>(&mut reader)
        .await
        .expect("empty preamble must preserve bytes read ahead");
    assert_eq!(leftover, [9], "leftover should equal [9]");
    assert_eq!(reader.position(), 1, "reader.position() should equal 1");
}
