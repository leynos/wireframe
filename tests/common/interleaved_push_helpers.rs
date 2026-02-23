//! Shared helpers for interleaved push queue tests.
//!
//! Used by both the rstest unit tests
//! (`tests/interleaved_push_queues.rs`) and the BDD fixture
//! (`tests/fixtures/interleaved_push_queues.rs`) to avoid duplicating
//! actor setup and execution boilerplate.

use std::future::Future;

use tokio_util::sync::CancellationToken;
use wireframe::{
    connection::{ConnectionActor, FairnessConfig},
    push::{PushHandle, PushQueues},
};
use wireframe_testing::TestResult;

/// Build unlimited queues, load frames via `setup`, run the actor with
/// the given `fairness` config, and return the collected output.
pub async fn run_actor_with_fairness<S, SFut>(
    fairness: FairnessConfig,
    setup: S,
) -> TestResult<Vec<u8>>
where
    S: FnOnce(PushHandle<u8>) -> SFut,
    SFut: Future<Output = ()>,
{
    let (queues, handle) = PushQueues::<u8>::builder()
        .high_capacity(8)
        .low_capacity(8)
        .unlimited()
        .build()?;

    setup(handle.clone()).await;

    let shutdown = CancellationToken::new();
    let mut actor: ConnectionActor<_, ()> = ConnectionActor::new(queues, handle, None, shutdown);
    actor.set_fairness(fairness);
    let mut out = Vec::new();
    actor
        .run(&mut out)
        .await
        .map_err(|e| format!("actor run failed: {e:?}"))?;
    Ok(out)
}
