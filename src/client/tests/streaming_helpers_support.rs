//! Shared request builders for streaming helper tests.

use futures::TryStreamExt;

use super::streaming_infra::{
    CorrelationId,
    MessageId,
    Payload,
    TestStreamEnvelope,
    setup_streaming_test,
};
use crate::client::{ClientError, StreamingResponseExt};

pub(super) fn build_request(correlation_id: CorrelationId) -> TestStreamEnvelope {
    TestStreamEnvelope::data(MessageId::new(99), correlation_id, Payload::new(vec![]))
}

pub(super) async fn collect_typed_items<F>(
    cid: CorrelationId,
    frames: Vec<TestStreamEnvelope>,
    mapper: F,
) -> Result<Vec<Vec<u8>>, Box<dyn std::error::Error + Send + Sync>>
where
    F: FnMut(TestStreamEnvelope) -> Result<Option<Vec<u8>>, ClientError> + Send + Unpin + 'static,
{
    let (mut client, _server) = setup_streaming_test(frames).await?;
    let items: Vec<Vec<u8>> = client
        .call_streaming(build_request(cid))
        .await?
        .typed_with(mapper)
        .try_collect()
        .await?;
    Ok(items)
}
