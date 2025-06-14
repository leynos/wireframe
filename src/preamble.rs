use bincode::error::DecodeError;
use bincode::{Decode, config, decode_from_slice};
use tokio::io::{AsyncRead, AsyncReadExt};

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
pub async fn read_preamble<R, T>(reader: &mut R) -> Result<T, DecodeError>
where
    R: AsyncRead + Unpin,
    T: Decode<()>,
{
    let mut buf = Vec::new();
    let config = config::standard()
        .with_big_endian()
        .with_fixed_int_encoding();
    loop {
        match decode_from_slice::<T, _>(&buf, config) {
            Ok((value, _)) => return Ok(value),
            Err(DecodeError::UnexpectedEnd { additional }) => {
                let start = buf.len();
                buf.resize(start + additional, 0);
                reader
                    .read_exact(&mut buf[start..])
                    .await
                    .map_err(|inner| DecodeError::Io { inner, additional })?;
            }
            Err(e) => return Err(e),
        }
    }
}
