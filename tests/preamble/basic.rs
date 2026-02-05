//! Basic preamble parsing tests.

use tokio::io::{AsyncWriteExt, duplex};
use wireframe::preamble::read_preamble;

use crate::{common::TestResult, support::HotlinePreamble};

#[tokio::test]
#[expect(
    clippy::panic_in_result_fn,
    reason = "asserts provide clearer diagnostics in tests"
)]
async fn parse_valid_preamble() -> TestResult {
    let (mut client, mut server) = duplex(64);
    let bytes = b"TRTPHOTL\x00\x01\x00\x02";
    client.write_all(bytes).await?;
    client.shutdown().await?;
    let (p, _) = read_preamble::<_, HotlinePreamble>(&mut server).await?;
    p.validate()?;
    assert_eq!(p.magic, HotlinePreamble::MAGIC, "preamble magic mismatch");
    assert_eq!(p.min_version, 1, "preamble minimum version mismatch");
    assert_eq!(p.client_version, 2, "preamble client version mismatch");
    Ok(())
}

#[tokio::test]
#[expect(
    clippy::panic_in_result_fn,
    reason = "asserts provide clearer diagnostics in tests"
)]
async fn invalid_magic_is_error() -> TestResult {
    let (mut client, mut server) = duplex(64);
    let bytes = b"WRONGMAG\x00\x01\x00\x02";
    client.write_all(bytes).await?;
    client.shutdown().await?;
    let (preamble, _) = read_preamble::<_, HotlinePreamble>(&mut server).await?;
    assert!(
        preamble.validate().is_err(),
        "invalid magic should fail validation"
    );
    Ok(())
}
