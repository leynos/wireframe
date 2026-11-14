//! Property coverage for `before_send` across fragmented output.
//!
//! The example-based tests in the parent module pin individual boundaries.
//! These generated cases sweep frame budgets and payload lengths so the
//! "one hook call per emitted frame" invariant is checked across the fragment
//! cap rather than at a single hand-picked size.

use std::{
    num::NonZeroUsize,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use bytes::Bytes;
use proptest::{
    prelude::*,
    test_runner::{TestCaseError, TestCaseResult},
};
use tokio::runtime::Builder;

use super::*;

// Generate payload lengths and frame budgets that cross the fragment cap.
proptest! {
    #![proptest_config(ProptestConfig {
        cases: 32,
        .. ProptestConfig::default()
    })]

    /// Every frame emitted for a generated payload is mutated exactly once.
    ///
    /// `size_case` selects an empty, below-cap, at-cap, above-cap, or
    /// multi-fragment payload for each generated frame budget, so the boundary
    /// cases are exercised rather than left to chance. The hook must run once
    /// per emitted frame, including every fragment.
    #[test]
    fn fragmented_output_marks_every_frame_once(
        frame_budget in 96usize..=256,
        size_case in 0usize..=4,
        slack in 0usize..=8,
    ) {
        run_fragmented_property_case(frame_budget, size_case, slack)?;
    }
}

/// Drive one generated fragmentation case on a deterministic runtime.
fn run_fragmented_property_case(
    frame_budget: usize,
    size_case: usize,
    slack: usize,
) -> TestCaseResult {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| TestCaseError::fail(error.to_string()))?;
    runtime
        .block_on(exercise_fragmented_property_case(
            frame_budget,
            size_case,
            slack,
        ))
        .map_err(|error| TestCaseError::fail(error.to_string()))
}

/// Map a generated size case onto a payload length around the fragment cap.
///
/// The cases are empty, just below the cap, exactly at the cap, just above the
/// cap, and a multi-fragment message. `slack` varies the non-boundary cases so
/// the generator explores a range rather than a single value.
fn payload_len_for_case(cap: usize, size_case: usize, slack: usize) -> usize {
    match size_case {
        0 => 0,
        1 => cap.saturating_sub(1 + slack),
        2 => cap,
        3 => cap.saturating_add(1 + slack),
        _ => cap.saturating_mul(2).saturating_add(slack),
    }
}

/// Fragment a generated payload, flush it, and check every wire frame.
async fn exercise_fragmented_property_case(
    frame_budget: usize,
    size_case: usize,
    slack: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    let message_cap = NonZeroUsize::new(4096).ok_or("message cap must be non-zero")?;
    let config =
        FragmentationConfig::for_frame_budget(frame_budget, message_cap, Duration::from_secs(5))
            .ok_or("frame budget must exceed fragmentation overhead")?;
    let payload_len = payload_len_for_case(config.fragment_payload_cap.get(), size_case, slack);
    let counter = Arc::new(AtomicUsize::new(0));
    let mut pipeline: FramePipeline<Bytes> =
        FramePipeline::with_hooks(Some(config), marker_hooks(&counter));
    pipeline.process(Envelope::new(1, None, vec![7; payload_len]))?;

    let FramedPair {
        mut server,
        mut client,
    } = framed_pair();
    let codec = TestCodec::default();
    let mut transport = OutboundTransport {
        serializer: &BincodeSerializer,
        codec: &codec,
        sink: &mut server,
    };
    flush_pipeline_output(&mut transport, &mut pipeline).await?;
    drop(server);

    let frames = collect_frames(&mut client).await;
    if frames.is_empty() {
        return Err("fragmentation should emit at least one frame".into());
    }
    if !frames.iter().all(|frame| frame.ends_with(&[MARKER])) {
        return Err("before_send should mutate every emitted frame".into());
    }
    let calls = counter.load(Ordering::SeqCst);
    if calls != frames.len() {
        return Err(format!("before_send ran {calls} times for {} frames", frames.len()).into());
    }
    Ok(())
}
