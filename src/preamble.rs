//! Connection preamble encoding and decoding utilities.
//!
//! The optional preamble is exchanged before processing a connection. This
//! module provides helpers to encode and decode preambles using `bincode`.

use bincode::{
    BorrowDecode,
    Encode,
    borrow_decode_from_slice,
    config,
    encode_to_vec,
    error::{DecodeError, EncodeError},
};
use tokio::io::{self, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

const MAX_PREAMBLE_LEN: usize = 1024;

/// Trait bound for types accepted as connection preambles.
///
/// The bound allows decoding borrowed data for any lifetime without
/// requiring an external decoding context. `Sync` is required because
/// preamble values are shared by reference with asynchronous handlers.
pub trait Preamble: for<'de> BorrowDecode<'de, ()> + Send + Sync + 'static {}
impl<T> Preamble for T where for<'de> T: BorrowDecode<'de, ()> + Send + Sync + 'static {}

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
        let range_start = start + read;
        let range_end = start + additional;
        let chunk = buf
            .get_mut(range_start..range_end)
            .ok_or(DecodeError::Other("preamble buffer range invalid"))?;
        match reader.read(chunk).await {
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
    // Start with a single byte to ensure we don't block on preambles smaller
    // than an arbitrary initial chunk size.
    let mut buf = Vec::new();
    read_more(reader, &mut buf, 1).await?;
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

/// Asynchronously encode and write a connection preamble using bincode.
///
/// This helper encodes the preamble using the same configuration as
/// [`read_preamble`] (big-endian, fixed int encoding) and writes the
/// resulting bytes to the provided writer.
///
/// # Errors
///
/// Returns an [`EncodeError`] if encoding fails, or wraps an I/O error
/// if writing to `writer` fails.
///
/// # Examples
///
/// ```
/// use tokio::io::{AsyncReadExt, duplex};
/// use wireframe::preamble::{read_preamble, write_preamble};
///
/// #[derive(Debug, PartialEq, bincode::Encode, bincode::BorrowDecode)]
/// struct MyPreamble(u64);
///
/// #[tokio::main]
/// async fn main() {
///     let (mut client, mut server) = duplex(64);
///     write_preamble(&mut client, &MyPreamble(42))
///         .await
///         .expect("Failed to write preamble");
///     drop(client);
///
///     let (decoded, leftover) = read_preamble::<_, MyPreamble>(&mut server)
///         .await
///         .expect("Failed to read preamble");
///     assert_eq!(decoded.0, 42);
///     assert!(leftover.is_empty());
/// }
/// ```
pub async fn write_preamble<W, T>(writer: &mut W, preamble: &T) -> Result<(), EncodeError>
where
    W: AsyncWrite + Unpin,
    T: Encode,
{
    let config = config::standard()
        .with_big_endian()
        .with_fixed_int_encoding();
    let bytes = encode_to_vec(preamble, config)?;
    writer
        .write_all(&bytes)
        .await
        .map_err(|inner| EncodeError::Io {
            inner,
            index: bytes.len(),
        })?;
    writer.flush().await.map_err(|inner| EncodeError::Io {
        inner,
        index: bytes.len(),
    })?;
    Ok(())
}
