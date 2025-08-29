#![cfg(feature = "advanced-tests")]
//! Concurrency tests for push delivery using loom.
//!
//! These tests model concurrent push execution to validate fairness and
//! correct shutdown behaviour under various interleavings.

use loom::model;
use tokio::runtime::Builder;
use tokio_util::sync::CancellationToken;
use wireframe::{
    connection::ConnectionActor,
    push::PushQueues,
};

#[test]
fn concurrent_push_delivery() {
    model(|| {
        let rt = Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to build tokio runtime");

        rt.block_on(async {
            let (queues, handle) = PushQueues::<u8>::builder()
                .high_capacity(1)
                .low_capacity(1)
                .build()
                .expect("failed to build push queues");
            let token = CancellationToken::new();

            let out = loom::sync::Arc::new(loom::sync::Mutex::new(Vec::new()));
            let out_clone = out.clone();
            let mut actor: ConnectionActor<_, ()> =
                ConnectionActor::new(queues, handle.clone(), None, token.clone());

            let actor_task = tokio::spawn(async move {
                let mut buf = Vec::new();
                actor
                    .run(&mut buf)
                    .await
                    .expect("connection actor failed to run");
                out_clone
                    .lock()
                    .expect("mutex poisoned")
                    .extend(buf);
            });

            let h1 = handle.clone();
            let t1 = tokio::spawn(async move {
                h1
                    .push_high_priority(1u8)
                    .await
                    .expect("failed to push high priority frame");
            });

            let h2 = handle.clone();
            let t2 = tokio::spawn(async move {
                h2
                    .push_low_priority(2u8)
                    .await
                    .expect("failed to push low priority frame");
            });

            t1.await.expect("high priority task join failed");
            t2.await.expect("low priority task join failed");
            token.cancel();
            actor_task.await.expect("actor task join failed");

            let buf = out.lock().expect("mutex poisoned");
            assert!(buf.contains(&1));
            assert!(buf.contains(&2));
        });
    });
}

