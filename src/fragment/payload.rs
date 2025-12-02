//! Encoding helpers for fragment payloads carried inside envelopes.
//!
//! Fragments are embedded into an existing frame payload by prefixing a
//! short magic marker, the encoded [`FragmentHeader`], and finally the raw
//! fragment bytes. This keeps fragmentation transport-agnostic while letting
//! higher layers detect and strip fragment metadata before deserialising the
//! logical message.

use std::num::NonZeroUsize;

use bincode::{
    borrow_decode_from_slice,
    config,
    encode_to_vec,
    error::{DecodeError, EncodeError},
};

use super::{FragmentHeader, FragmentIndex, MessageId};

/// Magic prefix that marks an embedded fragment payload.
pub const FRAGMENT_MAGIC: &[u8; 4] = b"FRAG";

/// Fixed bytes required to wrap a fragment, excluding the fragment body.
///
/// # Panics
///
/// Panics if encoding a default [`FragmentHeader`] fails, which would indicate
/// a programmer error in the constant header definition.
#[must_use]
pub fn fragment_overhead() -> NonZeroUsize {
    // Encode a trivial header to determine the encoded size. The concrete
    // header size is stable for the fixed-width fields used here and must
    // remain well below `u16::MAX` to satisfy the framing format.
    let header = FragmentHeader::new(MessageId::new(0), FragmentIndex::zero(), false);
    let header_bytes = encode_to_vec(header, config::standard()).unwrap_or_else(|err| {
        panic!("fragment header encoding must be infallible for constants: {err}")
    });
    // Magic + length prefix (u16 big-endian) + encoded header.
    let overhead = FRAGMENT_MAGIC.len() + std::mem::size_of::<u16>() + header_bytes.len();
    NonZeroUsize::new(overhead).unwrap_or_else(|| {
        panic!("fragment overhead must be non-zero (computed {overhead})");
    })
}

/// Encode a fragment for transport by prefixing marker and header bytes.
///
/// The returned buffer layout is:
/// `[FRAGMENT_MAGIC][u16 header_len][header bytes][fragment payload]`.
///
/// # Errors
///
/// Returns a [`bincode::error::EncodeError`] if the header cannot be encoded.
pub fn encode_fragment_payload(
    header: FragmentHeader,
    payload: &[u8],
) -> Result<Vec<u8>, bincode::error::EncodeError> {
    let header_bytes = encode_to_vec(header, config::standard())?;
    let header_len = u16::try_from(header_bytes.len())
        .map_err(|_| EncodeError::Other("fragment header length must fit within u16::MAX"))?;

    let mut buf = Vec::with_capacity(
        FRAGMENT_MAGIC.len() + std::mem::size_of::<u16>() + header_bytes.len() + payload.len(),
    );
    buf.extend_from_slice(FRAGMENT_MAGIC);
    buf.extend_from_slice(&header_len.to_be_bytes());
    buf.extend_from_slice(&header_bytes);
    buf.extend_from_slice(payload);
    Ok(buf)
}

/// Attempt to decode a fragment payload.
///
/// Returns `Ok(Some((header, payload)))` when `payload` contains the fragment
/// marker and a valid encoded header, `Ok(None)` when the marker is absent, or
/// an error if the marker is present but decoding fails.
///
/// # Errors
///
/// Returns a [`DecodeError`] when the marker is present but the header bytes
/// cannot be decoded.
pub fn decode_fragment_payload(
    payload: &[u8],
) -> Result<Option<(FragmentHeader, &[u8])>, DecodeError> {
    let minimum_len = FRAGMENT_MAGIC.len() + std::mem::size_of::<u16>();
    if payload.len() < minimum_len {
        return Ok(None);
    }

    let Some(prefix) = payload.get(..FRAGMENT_MAGIC.len()) else {
        return Ok(None);
    };
    if prefix != FRAGMENT_MAGIC {
        return Ok(None);
    }

    let header_len_offset = FRAGMENT_MAGIC.len();
    let len_bytes = match (
        payload.get(header_len_offset),
        payload.get(header_len_offset + 1),
    ) {
        (Some(a), Some(b)) => [*a, *b],
        _ => {
            return Err(DecodeError::UnexpectedEnd {
                additional: minimum_len - payload.len(),
            });
        }
    };
    let header_len = u16::from_be_bytes(len_bytes) as usize;
    let header_start = header_len_offset + std::mem::size_of::<u16>();
    let header_end = header_start + header_len;

    let Some(header_bytes) = payload.get(header_start..header_end) else {
        return Err(DecodeError::UnexpectedEnd {
            additional: header_end.saturating_sub(payload.len()),
        });
    };

    if payload.len() < header_end {
        return Err(DecodeError::UnexpectedEnd {
            additional: header_end - payload.len(),
        });
    }

    let (header, consumed) =
        borrow_decode_from_slice::<FragmentHeader, _>(header_bytes, config::standard())?;
    if consumed != header_len {
        return Err(DecodeError::OtherString(
            "fragment header length mismatch".to_string(),
        ));
    }

    let remainder = payload.get(header_end..).unwrap_or_default();
    Ok(Some((header, remainder)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_fragment_payload() {
        let header = FragmentHeader::new(MessageId::new(9), FragmentIndex::new(2), true);
        let payload = [1_u8, 2, 3, 4];

        let encoded = encode_fragment_payload(header, &payload).expect("encode fragment");
        let decoded = decode_fragment_payload(&encoded)
            .expect("decode fragment")
            .expect("fragment marker present");
        assert_eq!(decoded.0, header);
        assert_eq!(decoded.1, payload);
    }

    #[test]
    fn decode_returns_none_for_non_fragment_payloads() {
        let payload = [0_u8, 1, 2, 3];
        assert!(
            decode_fragment_payload(&payload)
                .expect("decode ok")
                .is_none()
        );
    }

    #[test]
    fn fragment_overhead_matches_encoded_header() {
        let header = FragmentHeader::new(MessageId::new(1), FragmentIndex::zero(), true);
        let encoded = encode_to_vec(header, config::standard()).expect("encode header");
        let expected = FRAGMENT_MAGIC.len() + std::mem::size_of::<u16>() + encoded.len();
        assert_eq!(fragment_overhead().get(), expected);
        assert!(encoded.len() < u16::MAX as usize, "header must fit in u16");
    }

    #[test]
    fn decode_fragment_payload_rejects_truncated_header() {
        let header = FragmentHeader::new(MessageId::new(2), FragmentIndex::new(1), false);
        let encoded = encode_to_vec(header, config::standard()).expect("encode header");

        // Advertise a longer header than provided to force `UnexpectedEnd`.
        let advertised_len: u16 = (encoded.len() + 4)
            .try_into()
            .expect("encoded header length must stay within u16");
        let mut payload = Vec::new();
        payload.extend_from_slice(FRAGMENT_MAGIC);
        payload.extend_from_slice(&advertised_len.to_be_bytes());
        payload.extend_from_slice(&encoded);

        let err = decode_fragment_payload(&payload).expect_err("expected decode failure");
        match err {
            DecodeError::UnexpectedEnd { .. } => {}
            other => panic!("expected UnexpectedEnd, got {other:?}"),
        }
    }

    #[test]
    fn decode_fragment_payload_rejects_length_mismatch() {
        let header = FragmentHeader::new(MessageId::new(3), FragmentIndex::new(5), true);
        let mut encoded = encode_to_vec(header, config::standard()).expect("encode header");
        encoded.extend_from_slice(&[0_u8, 1]); // pad so the advertised length exceeds consumed.
        let advertised_len: u16 = encoded
            .len()
            .try_into()
            .expect("padded header length must fit in u16");

        let mut payload = Vec::new();
        payload.extend_from_slice(FRAGMENT_MAGIC);
        payload.extend_from_slice(&advertised_len.to_be_bytes());
        payload.extend_from_slice(&encoded);

        let err = decode_fragment_payload(&payload).expect_err("expected decode failure");
        match err {
            DecodeError::OtherString(msg) => {
                assert_eq!(msg, "fragment header length mismatch");
            }
            other => panic!("expected length mismatch error, got {other:?}"),
        }
    }
}
