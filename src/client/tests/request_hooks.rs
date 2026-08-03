//! Unit tests for client request hooks (`before_send` and `after_receive`).

use std::sync::Arc;

use rstest::rstest;

use super::request_hooks_support::{
    HookCounter,
    HookLog,
    hook_counter,
    hook_log,
    run_hook_test,
    run_hook_test_with_capture,
    send_and_receive_test_body,
    send_envelope_test_body,
};
use crate::{app::Envelope, correlation::CorrelatableFrame};

#[rstest]
#[tokio::test]
async fn before_send_hook_invoked_on_send(hook_counter: HookCounter) {
    run_hook_test(
        |b| b.before_send(hook_counter.before_send_hook()),
        |client| Box::pin(send_envelope_test_body(client)),
    )
    .await
    .expect("run hook test");

    hook_counter.assert_count(1, "before_send hook should be invoked once");
}

#[rstest]
#[tokio::test]
async fn after_receive_hook_invoked_on_receive(hook_counter: HookCounter) {
    run_hook_test(
        |b| b.after_receive(hook_counter.after_receive_hook()),
        |client| Box::pin(send_and_receive_test_body(client)),
    )
    .await
    .expect("run hook test");

    hook_counter.assert_count(1, "after_receive hook should be invoked once");
}

#[rstest]
#[case::before_send(true)]
#[case::after_receive(false)]
#[tokio::test]
async fn multiple_hooks_execute_in_registration_order(
    hook_log: HookLog,
    #[case] is_before_send: bool,
) {
    run_hook_test(
        |b| {
            if is_before_send {
                b.before_send(hook_log.before_send_hook(b'A'))
                    .before_send(hook_log.before_send_hook(b'B'))
            } else {
                b.after_receive(hook_log.after_receive_hook(b'A'))
                    .after_receive(hook_log.after_receive_hook(b'B'))
            }
        },
        |client| {
            if is_before_send {
                Box::pin(send_envelope_test_body(client))
            } else {
                Box::pin(send_and_receive_test_body(client))
            }
        },
    )
    .await
    .expect("run hook test");

    hook_log.assert_entries(b"AB", "hooks should execute in registration order");
}

#[tokio::test]
async fn both_hooks_fire_for_call_correlated() {
    let send_counter = HookCounter::new();
    let recv_counter = HookCounter::new();

    run_hook_test(
        |b| {
            b.before_send(send_counter.before_send_hook())
                .after_receive(recv_counter.after_receive_hook())
        },
        |client| {
            Box::pin(async move {
                let request = Envelope::new(1, None, vec![10, 20]);
                let _response: Envelope = client.call_correlated(request).await?;
                Ok(())
            })
        },
    )
    .await
    .expect("run hook test");

    send_counter.assert_count(1, "before_send fires");
    recv_counter.assert_count(1, "after_receive fires");
}

#[tokio::test]
async fn no_hooks_configured_works_identically() {
    let correlation_id = Arc::new(std::sync::Mutex::new(None));
    let cid = correlation_id.clone();

    run_hook_test(
        |b| b,
        |client| {
            Box::pin(async move {
                let request = Envelope::new(1, None, vec![5, 6, 7]);
                let response: Envelope = client.call_correlated(request).await?;
                *cid.lock().expect("lock") = response.correlation_id();
                Ok(())
            })
        },
    )
    .await
    .expect("run hook test");

    assert_eq!(
        *correlation_id.lock().expect("lock"),
        Some(1),
        "correlation ID should match without hooks"
    );
}

#[derive(bincode::Encode, bincode::BorrowDecode)]
struct Ping(u8);

#[rstest]
#[tokio::test]
async fn before_send_hook_fires_for_plain_send(hook_counter: HookCounter) {
    let _captured = run_hook_test_with_capture(
        |b| b.before_send(hook_counter.before_send_hook()),
        |client| {
            Box::pin(async move {
                // Use the plain send() API (not envelope-aware).
                client.send(&Ping(42)).await?;
                Ok(())
            })
        },
    )
    .await
    .expect("run hook test");

    hook_counter.assert_count(1, "before_send should fire for plain send()");
}

#[tokio::test]
async fn before_send_hook_can_mutate_frame_bytes_on_wire() {
    const MARKER: u8 = 0xff;

    let captured = run_hook_test_with_capture(
        |b| {
            b.before_send(move |bytes: &mut Vec<u8>| {
                bytes.push(MARKER);
            })
        },
        |client| Box::pin(send_envelope_test_body(client)),
    )
    .await
    .expect("run hook test");

    let frame = captured
        .first()
        .expect("server should capture exactly one frame");
    assert_eq!(
        frame.last().copied(),
        Some(MARKER),
        "marker byte appended by before_send hook should be visible on the wire"
    );
}

#[tokio::test]
async fn after_receive_hook_can_mutate_frame_bytes_before_deserialization() {
    // Pre-serialize a replacement envelope with a distinctive payload.
    let replacement = Envelope::new(42, Some(1), vec![99, 98, 97]);
    let replacement_bytes =
        bincode::encode_to_vec(&replacement, bincode::config::standard()).expect("encode");

    let hook = move |bytes: &mut bytes::BytesMut| {
        bytes.clear();
        bytes.extend_from_slice(&replacement_bytes);
    };

    run_hook_test(
        |b| b.after_receive(hook),
        |client| {
            Box::pin(async move {
                let envelope = Envelope::new(1, None, vec![1, 2, 3]);
                client.send_envelope(envelope).await?;
                let response: Envelope = client.receive_envelope().await?;
                assert_eq!(
                    response.payload_bytes(),
                    &[99, 98, 97],
                    "after_receive hook mutation should be reflected in deserialized envelope"
                );
                Ok(())
            })
        },
    )
    .await
    .expect("run hook test");
}
