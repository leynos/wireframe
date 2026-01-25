//! Fragment rejection tests for malformed, out-of-order, and duplicate fragments.
//!
//! Verifies that the fragment transport layer correctly rejects invalid fragment
//! sequences and prevents them from reaching the application handler.

use std::time::Duration;

use rstest::rstest;
use tokio::{sync::mpsc, time::timeout};
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use wireframe::{
    app::{Envelope, Packet, PacketParts},
    fragment::{FRAGMENT_MAGIC, FragmentationConfig, Fragmenter},
};

use crate::common::{
    TestResult,
    fragment_helpers::{
        CORRELATION,
        ROUTE_ID,
        TestError,
        fragment_envelope,
        fragmentation_config,
        make_app,
        send_envelopes,
        spawn_app,
    },
};

/// Test setup holding client, server, fragments, and receiver channel.
pub struct FragmentRejectionSetup {
    pub client: Framed<tokio::io::DuplexStream, LengthDelimitedCodec>,
    pub server: tokio::task::JoinHandle<std::io::Result<()>>,
    pub fragments: Vec<Envelope>,
    pub rx: mpsc::UnboundedReceiver<Vec<u8>>,
}

impl FragmentRejectionSetup {
    /// Create a new rejection test setup with the given fragment mutator.
    pub fn new(
        capacity: usize,
        config: FragmentationConfig,
        fragment_mutator: impl FnOnce(Vec<Envelope>) -> TestResult<Vec<Envelope>>,
    ) -> TestResult<Self> {
        let (tx, rx) = mpsc::unbounded_channel();
        let app = make_app(capacity, config, &tx)?;
        let (client, server) = spawn_app(app);
        let fragmenter = Fragmenter::new(config.fragment_payload_cap);

        let payload = vec![1_u8; 800];
        let request = Envelope::new(ROUTE_ID, CORRELATION, payload);
        let fragments = fragment_mutator(fragment_envelope(&request, &fragmenter)?)?;

        Ok(Self {
            client,
            server,
            fragments,
            rx,
        })
    }
}

/// Execute a fragment rejection test with the given mutator.
pub async fn test_fragment_rejection<F>(fragment_mutator: F, rejection_message: &str) -> TestResult
where
    F: FnOnce(Vec<Envelope>) -> TestResult<Vec<Envelope>>,
{
    let buffer_capacity = 512;
    let config = fragmentation_config(buffer_capacity)?;
    let FragmentRejectionSetup {
        mut client,
        server,
        fragments,
        mut rx,
    } = FragmentRejectionSetup::new(buffer_capacity, config, fragment_mutator)?;

    send_envelopes(&mut client, &fragments).await?;
    tokio::io::AsyncWriteExt::shutdown(client.get_mut()).await?;

    if let Ok(Some(_)) = timeout(Duration::from_millis(200), rx.recv()).await {
        return Err(TestError::Assertion(rejection_message.to_string()).into());
    }

    drop(client);
    server.await??;

    Ok(())
}

/// Type alias for fragment mutator functions.
pub type FragmentMutator = fn(Vec<Envelope>) -> TestResult<Vec<Envelope>>;

/// Mutate fragments by swapping the first two (out-of-order delivery).
pub fn mutate_out_of_order(mut fragments: Vec<Envelope>) -> TestResult<Vec<Envelope>> {
    if fragments.len() < 2 {
        return Err(TestError::Setup("expected at least two fragments").into());
    }

    fragments.swap(0, 1);
    Ok(fragments)
}

/// Mutate fragments by duplicating the first fragment.
pub fn mutate_duplicate(mut fragments: Vec<Envelope>) -> TestResult<Vec<Envelope>> {
    let duplicate = fragments
        .first()
        .cloned()
        .ok_or(TestError::Setup("fragmenter produced no fragments"))?;
    fragments.insert(1, duplicate);
    Ok(fragments)
}

/// Mutate fragments by truncating the header of the first fragment.
#[expect(
    clippy::panic_in_result_fn,
    reason = "asserts provide clearer diagnostics in tests"
)]
pub fn mutate_malformed_header(mut fragments: Vec<Envelope>) -> TestResult<Vec<Envelope>> {
    let parts = fragments
        .first()
        .cloned()
        .ok_or(TestError::Setup(
            "fragmenter must produce at least one fragment",
        ))?
        .into_parts();
    let mut payload = parts.clone().payload();
    assert!(
        payload.starts_with(FRAGMENT_MAGIC),
        "expected fragment to start with marker"
    );
    let truncate_len = FRAGMENT_MAGIC.len() + 2;
    if payload.len() > truncate_len {
        payload.truncate(truncate_len);
    } else {
        while payload.len() < truncate_len {
            payload.push(0);
        }
    }
    if let Some(first) = fragments.get_mut(0) {
        *first = Envelope::from_parts(PacketParts::new(
            parts.id(),
            parts.correlation_id(),
            payload,
        ));
    } else {
        return Err(TestError::Setup("fragment list unexpectedly empty").into());
    }
    fragments.truncate(1);
    Ok(fragments)
}

#[rstest]
#[case::out_of_order(
    mutate_out_of_order,
    "handler should not receive out-of-order fragments"
)]
#[case::duplicate(
    mutate_duplicate,
    "handler should not receive after duplicate fragment"
)]
#[case::malformed(mutate_malformed_header, "malformed fragment header is rejected")]
#[tokio::test]
async fn fragment_rejection_cases(
    #[case] mutator: FragmentMutator,
    #[case] rejection_message: &str,
) -> TestResult {
    test_fragment_rejection(mutator, rejection_message).await
}
