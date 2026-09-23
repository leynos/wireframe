//! Basic preamble parsing tests.

use tokio::io::{AsyncWriteExt, duplex};
use wireframe::preamble::read_preamble;
use wireframe_testing::TestResult;

use crate::support::HotlinePreamble;

#[tokio::test]
async fn parse_valid_preamble() -> TestResult {
    let (mut client, mut server) = duplex(64);
    let bytes = b"TRTPHOTL\x00\x01\x00\x02";
    client.write_all(bytes).await?;
    client.shutdown().await?;
    let (p, _) = read_preamble::<_, HotlinePreamble>(&mut server).await?;
    p.validate()?;
    if p.magic != HotlinePreamble::MAGIC {
        return Err(format!(
            "preamble magic mismatch: expected {:?}, got {:?}",
            HotlinePreamble::MAGIC,
            p.magic
        )
        .into());
    }
    if p.min_version != 1 {
        return Err(format!(
            "preamble minimum version mismatch: expected 1, got {}",
            p.min_version
        )
        .into());
    }
    if p.client_version != 2 {
        return Err(format!(
            "preamble client version mismatch: expected 2, got {}",
            p.client_version
        )
        .into());
    }
    Ok(())
}

#[tokio::test]
async fn invalid_magic_is_error() -> TestResult {
    let (mut client, mut server) = duplex(64);
    let bytes = b"WRONGMAG\x00\x01\x00\x02";
    client.write_all(bytes).await?;
    client.shutdown().await?;
    let (preamble, _) = read_preamble::<_, HotlinePreamble>(&mut server).await?;
    if preamble.validate().is_ok() {
        return Err("invalid magic should fail validation".into());
    }
    Ok(())
}
