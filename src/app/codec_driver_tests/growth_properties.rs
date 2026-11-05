//! Property coverage for `before_send` frame growth against the codec limit.
//!
//! The pipeline contract allows hook mutations up to the codec's
//! `max_frame_length`. These generated cases sweep growth amounts on both sides
//! of that boundary, so the accepted-and-delivered and rejected-without-write
//! behaviours are checked together rather than at a single fixed size.

use bytes::Bytes;
use proptest::{
    prelude::*,
    test_runner::{TestCaseError, TestCaseResult},
};
use rstest::rstest;
use tokio::runtime::Builder;

use super::*;

/// Frame limit shared by the growth cases.
///
/// [`MIN_FRAME_LENGTH`] is the smallest limit the codec accepts, and the
/// serialized envelope leaves roughly half of it spare, so generated growth
/// reaches both the accepted and rejected sides of the boundary.
const GROWTH_LIMIT: usize = MIN_FRAME_LENGTH;

// Generate hook growth spanning the codec frame limit.
proptest! {
    #![proptest_config(ProptestConfig {
        cases: 32,
        .. ProptestConfig::default()
    })]

    /// Hook growth is accepted up to the frame limit and refused beyond it.
    ///
    /// The generator places growth relative to the measured headroom, so every
    /// run samples both sides of the boundary. Growth that keeps the frame
    /// within [`GROWTH_LIMIT`] must be written in full. Growth past the limit
    /// must fail the send and leave nothing on the wire, rather than
    /// delivering a truncated or oversized frame.
    #[test]
    fn hook_growth_respects_the_codec_limit(delta in -8i64..=8i64) {
        run_growth_property_case(delta)?;
    }
}

/// Pin each boundary class so the three outcomes are not sampled by chance.
///
/// The property above sweeps offsets around the headroom, but a run need not
/// draw an offset that lands exactly on the limit. These cases make the
/// below, exactly-at, and above classes deterministic.
#[rstest]
#[case(-1, "below the limit")]
#[case(0, "exactly at the limit")]
#[case(1, "above the limit")]
fn hook_growth_boundary_classes(#[case] delta: i64, #[case] class: &str) -> TestCaseResult {
    run_growth_property_case(delta)
        .map_err(|error| TestCaseError::fail(format!("{class}: {error}")))
}

/// Drive one generated growth case on a deterministic runtime.
fn run_growth_property_case(delta: i64) -> TestCaseResult {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| TestCaseError::fail(error.to_string()))?;
    runtime
        .block_on(exercise_growth_property_case(delta))
        .map_err(|error| TestCaseError::fail(error.to_string()))
}

/// Grow one encoded frame around the headroom and check the send outcome.
async fn exercise_growth_property_case(delta: i64) -> Result<(), Box<dyn std::error::Error>> {
    let envelope = Envelope::new(1, None, vec![1]);
    // The codec wraps payloads unchanged, so the serialized length is the
    // frame length before the hook runs.
    let base_len = BincodeSerializer
        .serialize(&envelope)
        .map_err(|error| error.to_string())?
        .len();
    let extra = growth_for_delta(base_len, delta)?;
    let codec = TestCodec::new(GROWTH_LIMIT);
    let (client, server) = tokio::io::duplex(1024);
    let mut server_framed =
        Framed::new(server, CombinedCodec::new(codec.decoder(), codec.encoder()));
    let mut client_framed =
        Framed::new(client, CombinedCodec::new(codec.decoder(), codec.encoder()));

    let mut pipeline: FramePipeline<Bytes> = FramePipeline::with_hooks(None, growing_hooks(extra));
    pipeline.process(envelope)?;
    let mut transport = OutboundTransport {
        serializer: &BincodeSerializer,
        codec: &codec,
        framed: &mut server_framed,
    };
    let result = flush_pipeline_output(&mut transport, &mut pipeline).await;
    drop(server_framed);

    let frames = collect_frames(&mut client_framed).await;
    let expected_len = base_len
        .checked_add(extra)
        .ok_or("grown frame length overflowed")?;

    if expected_len > GROWTH_LIMIT {
        return check_oversized_growth(&result, &frames);
    }
    result?;
    if frames.len() != 1 {
        return Err(format!("expected one frame, received {}", frames.len()).into());
    }
    let written = frames.first().ok_or("one frame should reach the wire")?;
    if written.len() != expected_len {
        return Err(format!(
            "written frame is {} bytes, expected {expected_len}",
            written.len()
        )
        .into());
    }
    let grown = written.get(base_len..).unwrap_or_default();
    if extra > 0 && !grown.iter().all(|byte| *byte == MARKER) {
        return Err("the hook's growth did not reach the wire".into());
    }
    Ok(())
}

/// Convert a signed offset from the headroom into a non-negative growth size.
///
/// The headroom is the number of bytes the frame may still grow before it
/// exceeds [`GROWTH_LIMIT`]. Offsetting from it keeps every generated case
/// close to the boundary, so both the accepted and rejected sides are
/// exercised on each run.
fn growth_for_delta(base_len: usize, delta: i64) -> Result<usize, Box<dyn std::error::Error>> {
    let headroom = i64::try_from(GROWTH_LIMIT)
        .map_err(|error| error.to_string())?
        .checked_sub(i64::try_from(base_len).map_err(|error| error.to_string())?)
        .ok_or("frame length exceeds the codec limit before the hook runs")?;
    let extra = headroom
        .checked_add(delta)
        .ok_or("growth offset overflowed")?
        .max(0);
    Ok(usize::try_from(extra).map_err(|error| error.to_string())?)
}

/// Check that growth past the codec limit failed without writing a frame.
fn check_oversized_growth(
    result: &std::io::Result<()>,
    frames: &[Bytes],
) -> Result<(), Box<dyn std::error::Error>> {
    if result.is_ok() {
        return Err("a frame over the codec limit was accepted".into());
    }
    if !frames.is_empty() {
        return Err("an oversized frame reached the wire".into());
    }
    Ok(())
}
