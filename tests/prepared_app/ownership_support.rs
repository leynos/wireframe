//! Fixtures that make application-owned values observable from a test.
//!
//! Every value a prepared application owns outright is given an identity and a
//! drop ledger. The serializer and assembler additionally record the address
//! they were called through, using atomics only, so a test can prove that two
//! overlapping connections reached the same allocation rather than a
//! per-connection clone.

use std::{
    error::Error,
    fmt,
    io,
    sync::{
        Arc,
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use bytes::Bytes;
use wireframe::{
    app::{Envelope, Handler},
    frame::FrameMetadata,
    hooks::{ConnectionContext, WireframeProtocol},
    message::{DecodeWith, DeserializeContext, EncodeWith},
    message_assembler::{MessageAssembler, ParsedFrameHeader},
    push::PushHandle,
    serializer::{BincodeSerializer, MessageCompatibilitySerializer, Serializer},
    test_helpers::TestAssembler,
};

/// Identity of the serializer probe.
pub(super) const SERIALIZER_ID: u64 = 100;
/// Identity of the message assembler probe.
pub(super) const ASSEMBLER_ID: u64 = 101;
/// Identity of the setup callback probe.
pub(super) const SETUP_ID: u64 = 102;
/// Identity of the teardown callback probe.
pub(super) const TEARDOWN_ID: u64 = 103;
/// Identity of the handler probe.
pub(super) const HANDLER_ID: u64 = 104;
/// Identity of the protocol probe.
pub(super) const PROTOCOL_ID: u64 = 105;

/// Shared ledger recording which identities have been dropped.
///
/// A mutex is acceptable here because the ledger observes the application from
/// outside; no application-owned value is ever locked.
#[derive(Clone, Default)]
pub(super) struct DropLedger(Arc<Mutex<Vec<u64>>>);

impl DropLedger {
    /// Record one drop.
    pub(super) fn record(&self, id: u64) {
        let mut dropped = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        dropped.push(id);
    }

    /// Identities dropped so far, in drop order.
    pub(super) fn dropped(&self) -> Vec<u64> {
        let dropped = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        dropped.clone()
    }

    /// Whether no identity has been dropped yet.
    pub(super) fn is_empty(&self) -> bool {
        let dropped = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        dropped.is_empty()
    }
}

impl fmt::Debug for DropLedger {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("DropLedger").field(&self.dropped()).finish()
    }
}

/// Non-`Clone`, non-`Copy` value whose identity is observable when dropped.
pub(super) struct DropProbe {
    id: u64,
    ledger: DropLedger,
}

impl DropProbe {
    /// Create a probe that records `id` when dropped.
    pub(super) fn new(ledger: DropLedger, id: u64) -> Self { Self { id, ledger } }

    /// Return the probe's identity.
    pub(super) const fn id(&self) -> u64 { self.id }
}

impl Drop for DropProbe {
    fn drop(&mut self) { self.ledger.record(self.id); }
}

/// Address and call observability for the serializer and assembler probes.
///
/// Every field is an atomic so a probe can be shared by concurrent connection
/// tasks without introducing a lock inside the application-owned value.
///
/// The first observed address doubles as the "unset" marker, so an observed
/// address of zero would be indistinguishable from no call at all. Zero is
/// never a live object's address, so the recorder documents that assumption
/// rather than carrying a separate initialized flag.
#[derive(Clone, Default)]
pub(super) struct CallRecorder {
    calls: Arc<AtomicUsize>,
    first_address: Arc<AtomicUsize>,
    saw_distinct_address: Arc<AtomicBool>,
}

impl CallRecorder {
    /// Number of times the probed method has run.
    pub(super) fn calls(&self) -> usize { self.calls.load(Ordering::SeqCst) }

    /// The single address every call observed, if all calls agreed.
    ///
    /// Returns `None` before the first call and whenever two distinct
    /// addresses were observed.
    pub(super) fn single_address(&self) -> Option<usize> {
        let saw_one_address = self.calls.load(Ordering::SeqCst) > 0
            && !self.saw_distinct_address.load(Ordering::SeqCst);
        saw_one_address.then(|| self.first_address.load(Ordering::SeqCst))
    }

    /// Record one call and the address of the value it ran through.
    fn observe(&self, address: usize) {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let first =
            self.first_address
                .compare_exchange(0, address, Ordering::SeqCst, Ordering::SeqCst);
        if let Err(observed) = first
            && observed != address
        {
            self.saw_distinct_address.store(true, Ordering::SeqCst);
        }
    }
}

/// Non-zero-sized serializer that records its address and drops once.
pub(super) struct ProbeSerializer {
    id: u64,
    ledger: DropLedger,
    recorder: CallRecorder,
}

impl ProbeSerializer {
    /// Create a serializer probe identified by `id`.
    pub(super) fn new(id: u64, ledger: DropLedger) -> Self {
        Self {
            id,
            ledger,
            recorder: CallRecorder::default(),
        }
    }

    /// Shared call observability for this serializer.
    pub(super) fn recorder(&self) -> CallRecorder { self.recorder.clone() }

    /// Address of this serializer allocation.
    fn address(&self) -> usize { std::ptr::from_ref(self) as usize }
}

impl Drop for ProbeSerializer {
    fn drop(&mut self) { self.ledger.record(self.id); }
}

impl MessageCompatibilitySerializer for ProbeSerializer {}

impl Serializer for ProbeSerializer {
    fn serialize<M>(&self, value: &M) -> Result<Vec<u8>, Box<dyn Error + Send + Sync>>
    where
        M: EncodeWith<Self>,
    {
        self.recorder.observe(self.address());
        value.encode_with(self)
    }

    fn deserialize<M>(&self, bytes: &[u8]) -> Result<(M, usize), Box<dyn Error + Send + Sync>>
    where
        M: DecodeWith<Self>,
    {
        self.recorder.observe(self.address());
        M::decode_with(self, bytes, &DeserializeContext::empty())
    }

    fn deserialize_with_context<M>(
        &self,
        bytes: &[u8],
        context: &DeserializeContext<'_>,
    ) -> Result<(M, usize), Box<dyn Error + Send + Sync>>
    where
        M: DecodeWith<Self>,
    {
        self.recorder.observe(self.address());
        M::decode_with(self, bytes, context)
    }

    fn should_deserialize_after_parse(&self) -> bool { false }
}

impl FrameMetadata for ProbeSerializer {
    type Frame = Envelope;
    type Error = bincode::error::DecodeError;

    fn parse(&self, src: &[u8]) -> Result<(Self::Frame, usize), Self::Error> {
        self.recorder.observe(self.address());
        BincodeSerializer.parse(src)
    }
}

/// Non-zero-sized assembler that records its address and drops once.
pub(super) struct ProbeAssembler {
    id: u64,
    ledger: DropLedger,
    recorder: CallRecorder,
}

impl ProbeAssembler {
    /// Create an assembler probe identified by `id`.
    pub(super) fn new(id: u64, ledger: DropLedger) -> Self {
        Self {
            id,
            ledger,
            recorder: CallRecorder::default(),
        }
    }

    /// Shared call observability for this assembler.
    pub(super) fn recorder(&self) -> CallRecorder { self.recorder.clone() }
}

impl Drop for ProbeAssembler {
    fn drop(&mut self) { self.ledger.record(self.id); }
}

impl MessageAssembler for ProbeAssembler {
    fn parse_frame_header(&self, payload: &[u8]) -> io::Result<ParsedFrameHeader> {
        self.recorder.observe(std::ptr::from_ref(self) as usize);
        TestAssembler.parse_frame_header(payload)
    }
}

/// Protocol implementation that counts connection setups and drops once.
pub(super) struct ProbeProtocol {
    id: u64,
    ledger: DropLedger,
    setups: Arc<AtomicUsize>,
}

impl ProbeProtocol {
    /// Create a protocol probe identified by `id`.
    pub(super) fn new(id: u64, ledger: DropLedger, setups: Arc<AtomicUsize>) -> Self {
        Self { id, ledger, setups }
    }
}

impl Drop for ProbeProtocol {
    fn drop(&mut self) { self.ledger.record(self.id); }
}

impl WireframeProtocol for ProbeProtocol {
    type Frame = Bytes;
    type ProtocolError = ();

    fn on_connection_setup(&self, _handle: PushHandle<Self::Frame>, _ctx: &mut ConnectionContext) {
        self.setups.fetch_add(1, Ordering::SeqCst);
    }
}

/// Build a route handler that shares one allocation across every route id.
///
/// The captured probe is held only by the handler closure, so the ledger
/// records a drop exactly when the last clone of that one allocation is
/// released.
pub(super) fn probe_handler(
    ledger: DropLedger,
    probe_id: u64,
    calls: Arc<AtomicUsize>,
) -> Handler<Envelope> {
    let probe = Arc::new(DropProbe::new(ledger, probe_id));
    Arc::new(move |_envelope: &Envelope| {
        calls.fetch_add(1, Ordering::SeqCst);
        let _keep_alive = probe.id();
        Box::pin(async move {})
    })
}

/// Sort identities so a test can compare creation and drop multisets.
pub(super) fn sorted(mut ids: Vec<u64>) -> Vec<u64> {
    ids.sort_unstable();
    ids
}
