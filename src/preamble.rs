//! Connection preamble decoding utilities.
//!
//! The optional preamble is read before processing a connection, and this
//! module provides helpers to decode it using `bincode`.

use bincode::{BorrowDecode, borrow_decode_from_slice, config, error::DecodeError};
use tokio::io::{self, AsyncRead, AsyncReadExt};

const MAX_PREAMBLE_LEN: usize = 1024;

/// Trait bound for types accepted as connection preambles.
///
/// The bound allows decoding borrowed data for any lifetime without
/// requiring an external decoding context.
pub trait Preamble: for<'de> BorrowDecode<'de, ()> + Send + 'static {}
impl<T> Preamble for T where for<'de> T: BorrowDecode<'de, ()> + Send + 'static {}

async fn read_more<R>(
    reader: &mut R,
    buf: &mut Vec<u8>,
    additional: usize,
) -> Result<(), DecodeError>
where
    R: AsyncRead + Unpin,
{
    let start = buf.len();
    if start + additional > MAX_PREAMBLE_LEN {
        return Err(DecodeError::Other("preamble too long"));
    }
    buf.resize(start + additional, 0);
    let mut read = 0;
    while read < additional {
        match reader
            .read(&mut buf[start + read..start + additional])
            .await
        {
            Ok(0) => {
                return Err(DecodeError::Io {
                    inner: io::Error::from(io::ErrorKind::UnexpectedEof),
                    additional: additional - read,
                });
            }
            Ok(n) => read += n,
            Err(inner) => {
                return Err(DecodeError::Io {
                    inner,
                    additional: additional - read,
                });
            }
        }
    }
    Ok(())
}

/// Asynchronously read and decode a connection preamble using bincode.
///
/// This helper reads the exact number of bytes required by `T`, as
/// indicated by [`DecodeError::UnexpectedEnd`]. Additional bytes are
/// requested from `reader` until decoding succeeds or fails for some
/// other reason.
///
/// Attempts to decode a value of type `T` from the beginning of the
/// byte stream, reading more bytes as needed until decoding succeeds or
/// an error occurs. Any bytes remaining after the decoded value are
/// returned as leftovers.
///
/// # Returns
///
/// A tuple containing the decoded value and any leftover bytes.
///
/// # Errors
///
/// Returns a [`DecodeError`] if decoding fails or if an I/O error occurs
/// while reading from `reader`.
///
/// # Examples
///
/// ```
/// use tokio::io::BufReader;
/// use wireframe::preamble::read_preamble;
///
/// #[derive(Debug, PartialEq, bincode::Encode, bincode::BorrowDecode)]
/// struct MyPreamble(u64);
///
/// #[tokio::main]
/// async fn main() {
///     let data = bincode::encode_to_vec(
///         MyPreamble(42),
///         bincode::config::standard()
///             .with_big_endian()
///             .with_fixed_int_encoding(),
///     )
///     .expect("Failed to encode example preamble");
///     let mut reader = BufReader::new(&data[..]);
///     let (preamble, leftover) = read_preamble::<_, MyPreamble>(&mut reader)
///         .await
///         .expect("Failed to decode preamble bytes");
///     assert_eq!(preamble.0, 42);
///     assert!(leftover.is_empty());
/// }
/// ```
pub async fn read_preamble<R, T>(reader: &mut R) -> Result<(T, Vec<u8>), DecodeError>
where
    R: AsyncRead + Unpin,
    // Decode borrowed data for any lifetime without external context.
    for<'de> T: BorrowDecode<'de, ()>,
{
    // Read a small chunk upfront to avoid a guaranteed decode failure on the
    // first iteration.
    let mut buf = Vec::new();
    read_more(reader, &mut buf, 8.min(MAX_PREAMBLE_LEN)).await?;
    // Build the decoder configuration once to avoid repeated allocations.
    let config = config::standard()
        .with_big_endian()
        .with_fixed_int_encoding();
    loop {
        match borrow_decode_from_slice::<T, _>(&buf, config) {
            Ok((value, consumed)) => {
                let leftover = buf.split_off(consumed);
                return Ok((value, leftover));
            }
            Err(DecodeError::UnexpectedEnd { additional }) => {
                read_more(reader, &mut buf, additional).await?;
            }
            Err(e) => return Err(e),
        }
    }
}
