use bincode::error::DecodeError;
use bincode::{BorrowDecode, borrow_decode_from_slice, config};
use tokio::io::{self, AsyncRead, AsyncReadExt};

const MAX_PREAMBLE_LEN: usize = 1024;

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

/// Read and decode a connection preamble using bincode.
///
/// This helper reads the exact number of bytes required by `T`, as
/// indicated by [`DecodeError::UnexpectedEnd`]. Additional bytes are
/// requested from the reader until decoding succeeds or fails for some
/// other reason.
///
/// # Errors
///
/// Returns a [`DecodeError`] if decoding the preamble fails or an
/// underlying I/O error occurs while reading from `reader`.
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
