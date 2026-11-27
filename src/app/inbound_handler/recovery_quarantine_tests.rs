//! Tests for what a quarantine pause must observe and purge.
//!
//! A pause is the only recovery action that mutates connection state while the
//! connection stays alive, so it owns the tests for shutdown interleaving and
//! for discarding state that expired while it ran. The decision tests live in
//! [`super`].

use std::{num::NonZeroUsize, time::Duration};

use super::*;
use crate::{
    app::Envelope,
    fragment::{
        FragmentHeader,
        FragmentIndex,
        FragmentationConfig,
        MessageId,
        encode_fragment_payload,
    },
    message_assembler::{
        EnvelopeId,
        EnvelopeRouting,
        FirstFrameHeader,
        FirstFrameInput,
        MessageKey,
    },
};

#[tokio::test]
async fn shutdown_ends_a_quarantine_pause_early() {
    let hook = RecordingHook::new(RecoveryPolicy::Quarantine, Duration::from_secs(3600));
    let mut coordinator = RecoveryCoordinator::new(Some(&hook), RecoveryConfig::default());
    let shutdown = CancellationToken::new();
    shutdown.cancel();
    let mut pipeline: FramePipeline<()> = FramePipeline::new(None);
    let mut message_assembly = None;
    let mut evicted = Vec::new();

    let result = tokio::time::timeout(
        Duration::from_secs(5),
        coordinator.apply(
            protocol_failure(),
            RecoveryState {
                pipeline: &mut pipeline,
                message_assembly: &mut message_assembly,
                evicted_streams: &mut evicted,
                shutdown: &shutdown,
            },
        ),
    )
    .await;

    assert!(
        matches!(result, Ok(None)),
        "a cancelled shutdown must end the pause without a disconnect"
    );
}

#[tokio::test]
async fn quarantine_pause_completes_without_shutdown() {
    let pause = Duration::from_millis(5);
    let hook = RecordingHook::new(RecoveryPolicy::Quarantine, pause);
    let mut coordinator = RecoveryCoordinator::new(Some(&hook), RecoveryConfig::default());
    let shutdown = CancellationToken::new();
    let mut pipeline: FramePipeline<()> = FramePipeline::new(None);
    let mut message_assembly = None;
    let mut evicted = Vec::new();

    let result = coordinator
        .apply(
            protocol_failure(),
            RecoveryState {
                pipeline: &mut pipeline,
                message_assembly: &mut message_assembly,
                evicted_streams: &mut evicted,
                shutdown: &shutdown,
            },
        )
        .await;

    assert!(result.is_none(), "quarantine keeps the connection alive");
}

/// Run one quarantine pause over `assembly` and report what it evicted.
///
/// Every purge case shares this setup: a coordinator whose hook selects a
/// quarantine, a pause of zero length, and the state the pause mutates. The
/// cases then differ only in the fixture and the assertion.
async fn apply_quarantine(
    assembly: Option<MessageAssemblyState>,
) -> (
    Option<std::io::Error>,
    Option<MessageAssemblyState>,
    Vec<crate::message_assembler::StreamingAssemblyId>,
) {
    let hook = RecordingHook::new(RecoveryPolicy::Quarantine, Duration::ZERO);
    let mut coordinator = RecoveryCoordinator::new(Some(&hook), RecoveryConfig::default());
    let shutdown = CancellationToken::new();
    let mut pipeline: FramePipeline<Envelope> = FramePipeline::new(None);
    let mut message_assembly = assembly;
    let mut evicted = Vec::new();
    let error = coordinator
        .apply(
            protocol_failure(),
            RecoveryState {
                pipeline: &mut pipeline,
                message_assembly: &mut message_assembly,
                evicted_streams: &mut evicted,
                shutdown: &shutdown,
            },
        )
        .await;
    (error, message_assembly, evicted)
}

/// A pause must discard partial fragments that expired while it ran.
///
/// A buffer left over from before the pause would otherwise be reassembled
/// with fragments read afterwards, so the connection would report a frame
/// assembled from two different eras of the stream.
#[tokio::test]
async fn quarantine_pause_purges_expired_partial_fragments() {
    let hook = RecordingHook::new(RecoveryPolicy::Quarantine, Duration::ZERO);
    let mut coordinator = RecoveryCoordinator::new(Some(&hook), RecoveryConfig::default());
    let shutdown = CancellationToken::new();
    let mut pipeline: FramePipeline<Envelope> = FramePipeline::new(Some(fragment_config()));
    buffer_partial_fragment(&mut pipeline);
    assert!(
        pipeline.fragmentation_buffered_bytes() > 0,
        "the fixture must leave a partial fragment buffered"
    );
    let mut message_assembly = None;
    let mut evicted = Vec::new();

    let result = coordinator
        .apply(
            protocol_failure(),
            RecoveryState {
                pipeline: &mut pipeline,
                message_assembly: &mut message_assembly,
                evicted_streams: &mut evicted,
                shutdown: &shutdown,
            },
        )
        .await;

    assert!(result.is_none(), "a quarantine pause keeps reading");
    assert_eq!(
        pipeline.fragmentation_buffered_bytes(),
        0,
        "the pause must purge the expired partial fragment"
    );
}

/// A pause must discard assemblies that expired while it ran.
///
/// Assembly state is separate from fragmentation state: a partial message whose
/// first frame arrived before the pause would otherwise be completed by
/// continuations read afterwards, splicing two eras of the stream together.
#[tokio::test]
async fn quarantine_pause_purges_expired_message_assemblies() {
    let fixture = Some(expired_assembly());
    assert_eq!(
        fixture.as_ref().map(MessageAssemblyState::buffered_count),
        Some(1),
        "the fixture must leave one incomplete message buffered"
    );

    let (error, assembly, _) = apply_quarantine(fixture).await;

    assert!(error.is_none(), "a quarantine pause keeps reading");
    assert_eq!(
        assembly.as_ref().map(MessageAssemblyState::buffered_count),
        Some(0),
        "the pause must purge the expired assembly"
    );
}

/// An assembly state holding one incomplete message that has already expired.
fn expired_assembly() -> MessageAssemblyState {
    incomplete_assembly(1, &[0x01, 0x02], Admission::Buffered)
}

/// An assembly state holding one streaming partial that has already expired.
///
/// A streaming first frame inserts a partial that retains only its ordering and
/// byte counts, which is the entry a pause must report rather than silently
/// discard.
fn streaming_assembly() -> MessageAssemblyState {
    incomplete_assembly(2, &[], Admission::Streaming)
}

/// Which admission a fixture's first frame goes through.
#[derive(Clone, Copy)]
enum Admission {
    /// The message is assembled in full before its route runs.
    Buffered,
    /// The partial is tracked for a streaming route and retains no payload.
    Streaming,
}

/// An assembly state holding one incomplete message, admitted by `kind`.
fn incomplete_assembly(key: u64, metadata: &[u8], kind: Admission) -> MessageAssemblyState {
    let mut state = MessageAssemblyState::new(
        NonZeroUsize::new(4096).expect("a non-zero message size limit"),
        Duration::ZERO,
    );
    let header = FirstFrameHeader {
        message_key: MessageKey(key),
        metadata_len: metadata.len(),
        body_len: 3,
        total_body_len: Some(10),
        is_last: false,
    };
    let routing = EnvelopeRouting {
        envelope_id: EnvelopeId(u32::try_from(key).unwrap_or(0)),
        correlation_id: None,
    };
    let input = FirstFrameInput::new(&header, routing, metadata.to_vec(), b"abc")
        .expect("the frame lengths match the header");
    match kind {
        Admission::Buffered => {
            state
                .accept_first_frame(input)
                .expect("the assembly accepts a first frame");
        }
        Admission::Streaming => {
            state
                .accept_streaming_first_frame(input)
                .expect("the streaming assembly accepts a first frame");
        }
    }
    state
}

/// Fragmentation whose partial buffers expire immediately, so a purge is
/// observable without waiting for a clock.
fn fragment_config() -> FragmentationConfig {
    FragmentationConfig::for_frame_budget(
        1024,
        NonZeroUsize::new(4096).expect("a non-zero message limit"),
        Duration::ZERO,
    )
    .expect("the frame budget supports fragmentation")
}

/// Feed the pipeline the first fragment of a two-fragment message.
fn buffer_partial_fragment(pipeline: &mut FramePipeline<Envelope>) {
    let header = FragmentHeader::new(MessageId::new(9), FragmentIndex::zero(), false);
    let payload = encode_fragment_payload(header, &[1_u8, 2]).expect("the fragment encodes");
    let partial = Envelope::new(9, None, payload);
    let state = pipeline
        .fragmentation_mut()
        .expect("the fixture enables fragmentation");
    let _ = state
        .reassemble(partial)
        .expect("the first fragment is buffered");
}

/// A pause must hand the loop every streaming body its purge evicted.
///
/// Failing a body writes its reply, which a pause cannot do, so the ids are
/// staged for the loop instead of being dropped with the assemblies. Losing
/// them leaves the handler waiting for chunks that can never arrive while the
/// connection stays up.
#[tokio::test]
async fn quarantine_pause_stages_the_streaming_bodies_it_evicts() {
    let (error, _, evicted) = apply_quarantine(Some(streaming_assembly())).await;

    assert!(error.is_none(), "a quarantine pause keeps reading");
    assert_eq!(
        evicted.len(),
        1,
        "the evicted streaming body must reach the loop rather than be dropped"
    );
}
