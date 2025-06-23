use wireframe::push::{PushError, PushPolicy, PushPriority, PushQueues};

#[tokio::test]
async fn frames_routed_to_correct_priority_queues() {
    let (mut queues, handle) = PushQueues::bounded(1);

    handle.push_low_priority(1u8).await.unwrap();
    handle.push_high_priority(2u8).await.unwrap();

    let high = queues.high_priority_rx.recv().await;
    let low = queues.low_priority_rx.recv().await;

    assert_eq!(high, Some(2));
    assert_eq!(low, Some(1));
}

#[tokio::test]
async fn try_push_respects_policy() {
    let (mut queues, handle) = PushQueues::bounded(1);

    handle.push_high_priority(1u8).await.unwrap();
    let result = handle.try_push(2u8, PushPriority::High, PushPolicy::ReturnErrorIfFull);
    assert!(result.is_err());

    // drain queue to allow new push
    let _ = queues.high_priority_rx.recv().await;
    handle.push_high_priority(3u8).await.unwrap();
    let last = queues.high_priority_rx.recv().await;
    assert_eq!(last, Some(3));
}

#[tokio::test]
async fn push_queues_error_on_closed() {
    let (queues, handle) = PushQueues::bounded(1);

    drop(queues.high_priority_rx);
    let res = handle.push_high_priority(42u8).await;
    assert!(matches!(res, Err(PushError::Closed)));

    drop(queues.low_priority_rx);
    let res = handle.push_low_priority(24u8).await;
    assert!(matches!(res, Err(PushError::Closed)));
}
