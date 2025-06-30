#![cfg(feature = "advanced-tests")]

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
            .unwrap();

        rt.block_on(async {
            let (queues, handle) = PushQueues::bounded(1, 1);
            let token = CancellationToken::new();

            let out = loom::sync::Arc::new(loom::sync::Mutex::new(Vec::new()));
            let out_clone = out.clone();
            let mut actor: ConnectionActor<_, ()> =
                ConnectionActor::new(queues, handle.clone(), None, token.clone());

            let actor_task = tokio::spawn(async move {
                let mut buf = Vec::new();
                actor.run(&mut buf).await.unwrap();
                out_clone.lock().unwrap().extend(buf);
            });

            let h1 = handle.clone();
            let t1 = tokio::spawn(async move {
                h1.push_high_priority(1u8).await.unwrap();
            });

            let h2 = handle.clone();
            let t2 = tokio::spawn(async move {
                h2.push_low_priority(2u8).await.unwrap();
            });

            t1.await.unwrap();
            t2.await.unwrap();
            token.cancel();
            actor_task.await.unwrap();

            let buf = out.lock().unwrap();
            assert!(buf.contains(&1));
            assert!(buf.contains(&2));
        });
    });
}

