//! Tests for the connection loop's memory-pressure exit.
//!
//! Memory pressure ends the connection before the loop reads again, so a
//! decoder failure buffered from an earlier pass would be lost behind the
//! pressure error. These tests drive the loop's own `step` and assert that the
//! pressure error is returned unchanged, the buffered failure is reported once
//! as a disconnect, and the hook is never asked for a policy it cannot apply.

use std::{io, num::NonZeroUsize, time::Duration};

use bytes::{Bytes, BytesMut};
#[cfg(feature = "metrics")]
use metrics_util::debugging::DebuggingRecorder;
use tokio_util::{
    codec::{Decoder, Encoder},
    sync::CancellationToken,
};

use super::*;
#[cfg(feature = "metrics")]
use crate::app::inbound_handler::recovery::test_support::policy_count;
use crate::{
    app::{
        BudgetBytes,
        inbound_handler::recovery::test_support::{RecordingHook, protocol_failure},
    },
    codec::RecoveryPolicy,
    message_assembler::{
        EnvelopeRouting,
        FirstFrameHeader,
        FirstFrameInput,
        MessageAssemblyState,
        MessageKey,
    },
    serializer::BincodeSerializer,
};

/// Frame type for the framed connection these tests build.
#[derive(Clone, Debug)]
struct ProbeFrame(Vec<u8>);

/// Codec that never decodes: no test here reads a frame.
#[derive(Clone, Debug)]
struct ProbeCodec;

/// Decoder that leaves the read buffer untouched.
#[derive(Clone, Debug)]
struct ProbeDecoder;

/// Encoder that writes nothing.
#[derive(Clone, Debug)]
struct ProbeEncoder;

impl Decoder for ProbeDecoder {
    type Item = ProbeFrame;
    type Error = io::Error;

    fn decode(&mut self, _src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        Ok(None)
    }
}

impl Encoder<ProbeFrame> for ProbeEncoder {
    type Error = io::Error;

    fn encode(&mut self, _item: ProbeFrame, _dst: &mut BytesMut) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl FrameCodec for ProbeCodec {
    type Frame = ProbeFrame;
    type Decoder = ProbeDecoder;
    type Encoder = ProbeEncoder;

    fn decoder(&self) -> Self::Decoder { ProbeDecoder }

    fn encoder(&self) -> Self::Encoder { ProbeEncoder }

    fn frame_payload(frame: &Self::Frame) -> &[u8] { frame.0.as_slice() }

    fn wrap_payload(&self, payload: Bytes) -> Self::Frame { ProbeFrame(payload.to_vec()) }

    fn max_frame_length(&self) -> usize { 64 }
}

/// Budget whose aggregate cap is small enough for the assembly to exceed.
fn pressure_budgets() -> MemoryBudgets {
    let cap = NonZeroUsize::new(64).expect("cap is non-zero");
    MemoryBudgets::new(
        BudgetBytes::new(cap),
        BudgetBytes::new(cap),
        BudgetBytes::new(cap),
    )
}

/// Assembly state holding more buffered bytes than the hard cap allows.
///
/// The first frame has no terminator, so the assembly stays open and keeps its
/// bytes; that is the state a connection reaches after buffering a fragmented
/// message and before it reads the next frame.
fn over_cap_assembly() -> MessageAssemblyState {
    let buffered = 128;
    let capacity = NonZeroUsize::new(buffered + 64).expect("the assembly capacity is non-zero");
    let mut state = MessageAssemblyState::new(capacity, Duration::from_secs(30));
    let body = vec![0_u8; buffered];
    let header = FirstFrameHeader {
        message_key: MessageKey(1),
        metadata_len: 0,
        body_len: buffered,
        total_body_len: None,
        is_last: false,
    };
    let input = FirstFrameInput::new(&header, EnvelopeRouting::default(), vec![], &body)
        .expect("the first frame input is valid");
    let assembled = state
        .accept_first_frame(input)
        .expect("the first frame is accepted");
    assert!(
        assembled.is_none(),
        "an unterminated assembly must stay buffered"
    );
    state
}

/// Drive the pressure exit twice through the loop's own `step`.
///
/// Both facts the loop needs are set directly: a read fills the pending slot
/// only while a push batch is served, and the pressure the following step
/// evaluates comes from state buffered by earlier frames.
fn drive_pressure_exit(hook: &RecordingHook) -> (io::Result<bool>, io::Result<bool>) {
    let shutdown = CancellationToken::new();
    let codec = ProbeCodec;
    let routes: HashMap<u32, HandlerService<Envelope>> = HashMap::new();
    // The read half is never polled: every step aborts before it waits.
    let (_client, server) = tokio::io::duplex(64);
    let context = StreamProcessingContext {
        routes: &routes,
        serializer: Arc::new(BincodeSerializer),
        codec: &codec,
        message_assembler: None,
        fragmentation: None,
        protocol_hooks: ProtocolHooks::default(),
        protocol: None,
        memory_budgets: Some(pressure_budgets()),
        read_timeout_ms: 50,
        shutdown: &shutdown,
        push_queues: PushQueuesBuilder::default(),
        session_registry: Arc::new(SessionRegistry::new()),
        peer_addr: None,
        recovery_hook: Some(hook),
        recovery_config: RecoveryConfig::default(),
    };
    let mut loop_ = ConnectionLoop::new(server, context).expect("the loop is built");
    loop_.inbound.pending = Some(DecodedItem::Failure(protocol_failure()));
    loop_.inbound.message_assembly = Some(over_cap_assembly());

    let first = futures::executor::block_on(loop_.step());
    let second = futures::executor::block_on(loop_.step());
    (first, second)
}

#[test]
fn a_pressure_abort_reports_the_buffered_failure_as_a_disconnect() {
    // The hook would have dropped this failure, but the connection is already
    // ending, so reporting a drop would describe an action that never ran.
    let hook = RecordingHook::new(RecoveryPolicy::Drop, Duration::from_millis(5));

    let (first, second) = drive_pressure_exit(&hook);

    let error = first.expect_err("pressure ends the connection");
    assert_eq!(
        error.kind(),
        io::ErrorKind::InvalidData,
        "the pressure error must surface unchanged"
    );
    assert_eq!(
        error.to_string(),
        "per-connection memory budget hard cap exceeded",
        "the loop must return the pressure error, not the decoder's"
    );
    let again = second.expect_err("pressure still ends the connection");
    assert_eq!(
        again.kind(),
        error.kind(),
        "the second pass must fail for the same reason"
    );
    assert_eq!(hook.error_types(), vec!["protocol"]);
    assert_eq!(
        hook.applied_policies(),
        vec![RecoveryPolicy::Disconnect],
        "the failure must be reported exactly once, as the disconnect it is"
    );
    assert_eq!(
        hook.consultations(),
        0,
        "a connection that is already ending must not consult the hook"
    );
}

#[cfg(feature = "metrics")]
#[test]
fn a_pressure_abort_records_one_disconnect_metric() {
    let hook = RecordingHook::new(RecoveryPolicy::Drop, Duration::from_millis(5));
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();

    // Only the report step records; the pressure abort itself records nothing.
    metrics::with_local_recorder(&recorder, || {
        drop(drive_pressure_exit(&hook));
    });

    let samples = snapshotter.snapshot().into_vec();
    assert_eq!(policy_count(&samples, "disconnect"), 1);
    assert_eq!(
        policy_count(&samples, "drop"),
        0,
        "a policy that was never applied must not be recorded"
    );
}
