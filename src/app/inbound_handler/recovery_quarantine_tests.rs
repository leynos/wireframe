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

    let result = tokio::time::timeout(
        Duration::from_secs(5),
        coordinator.apply(
            protocol_failure(),
            RecoveryState {
                pipeline: &mut pipeline,
                message_assembly: &mut message_assembly,
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

    let result = coordinator
        .apply(
            protocol_failure(),
            RecoveryState {
                pipeline: &mut pipeline,
                message_assembly: &mut message_assembly,
                shutdown: &shutdown,
            },
        )
        .await;

    assert!(result.is_none(), "quarantine keeps the connection alive");
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

    let result = coordinator
        .apply(
            protocol_failure(),
            RecoveryState {
                pipeline: &mut pipeline,
                message_assembly: &mut message_assembly,
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
    let hook = RecordingHook::new(RecoveryPolicy::Quarantine, Duration::ZERO);
    let mut coordinator = RecoveryCoordinator::new(Some(&hook), RecoveryConfig::default());
    let shutdown = CancellationToken::new();
    let mut pipeline: FramePipeline<Envelope> = FramePipeline::new(None);
    let mut message_assembly = Some(expired_assembly());
    assert_eq!(
        message_assembly
            .as_ref()
            .map(MessageAssemblyState::buffered_count),
        Some(1),
        "the fixture must leave one incomplete message buffered"
    );

    let result = coordinator
        .apply(
            protocol_failure(),
            RecoveryState {
                pipeline: &mut pipeline,
                message_assembly: &mut message_assembly,
                shutdown: &shutdown,
            },
        )
        .await;

    assert!(result.is_none(), "a quarantine pause keeps reading");
    assert_eq!(
        message_assembly
            .as_ref()
            .map(MessageAssemblyState::buffered_count),
        Some(0),
        "the pause must purge the expired assembly"
    );
}

/// An assembly state holding one incomplete message that has already expired.
fn expired_assembly() -> MessageAssemblyState {
    let mut state = MessageAssemblyState::new(
        NonZeroUsize::new(4096).expect("a non-zero message size limit"),
        Duration::ZERO,
    );
    let header = FirstFrameHeader {
        message_key: MessageKey(1),
        metadata_len: 2,
        body_len: 3,
        total_body_len: Some(10),
        is_last: false,
    };
    let routing = EnvelopeRouting {
        envelope_id: EnvelopeId(1),
        correlation_id: None,
    };
    let input = FirstFrameInput::new(&header, routing, vec![0x01, 0x02], b"abc")
        .expect("the frame lengths match the header");
    state
        .accept_first_frame(input)
        .expect("the assembly accepts a first frame");
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
