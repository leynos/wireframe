use wireframe::push::{PushError, PushPolicy, PushPriority, PushQueues};

#[tokio::test]
async fn frames_routed_to_correct_priority_queues() {
    let (mut queues, handle) = PushQueues::bounded(1, 1);

    handle.push_low_priority(1u8).await.unwrap();
    handle.push_high_priority(2u8).await.unwrap();

    let (prio1, frame1) = queues.recv().await.unwrap();
    let (prio2, frame2) = queues.recv().await.unwrap();

    assert_eq!(prio1, PushPriority::High);
    assert_eq!(frame1, 2);
    assert_eq!(prio2, PushPriority::Low);
    assert_eq!(frame2, 1);
}

#[tokio::test]
async fn try_push_respects_policy() {
    let (mut queues, handle) = PushQueues::bounded(1, 1);

    handle.push_high_priority(1u8).await.unwrap();
    let result = handle.try_push(2u8, PushPriority::High, PushPolicy::ReturnErrorIfFull);
    assert!(result.is_err());

    // drain queue to allow new push
    let _ = queues.recv().await;
    handle.push_high_priority(3u8).await.unwrap();
    let (_, last) = queues.recv().await.unwrap();
    assert_eq!(last, 3);
}

#[tokio::test]
async fn push_queues_error_on_closed() {
    let (queues, handle) = PushQueues::bounded(1, 1);

    let mut queues = queues;
    queues.close();
    let res = handle.push_high_priority(42u8).await;
    assert!(matches!(res, Err(PushError::Closed)));

    let res = handle.push_low_priority(24u8).await;
    assert!(matches!(res, Err(PushError::Closed)));
}
