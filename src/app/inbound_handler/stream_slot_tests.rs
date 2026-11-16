//! Unit tests for the connection-owned streaming slot.

use std::{future, time::Duration};

use super::{ActiveStream, PendingEvent, PendingSend};

#[tokio::test(start_paused = true)]
async fn a_stalled_admission_returns_control_after_the_read_timeout() {
    let mut stream = stalled_stream();

    let event = stream.drive_pending(Duration::from_millis(50)).await;

    assert!(
        matches!(event, PendingEvent::Idle),
        "a stalled admission should hand control back to the loop"
    );
}

#[tokio::test(start_paused = true)]
async fn a_resolved_handler_wins_over_a_stalled_admission() {
    let mut stream = stalled_stream();
    stream.handler = Box::pin(async { Some(b"reply".to_vec()) });

    match stream.drive_pending(Duration::from_secs(1)).await {
        PendingEvent::Handler(outcome) => {
            assert_eq!(outcome, Some(b"reply".to_vec()));
        }
        _ => panic!("the handler outcome should win over a stalled admission"),
    }
}

#[tokio::test(start_paused = true)]
async fn a_completed_admission_wins_over_a_stalled_handler() {
    let mut stream = stalled_stream();
    stream.pending = Some(PendingSend {
        future: Box::pin(async { Ok(()) }),
        is_last: true,
    });

    match stream.drive_pending(Duration::from_secs(1)).await {
        PendingEvent::Send(result) => assert!(result.is_ok()),
        _ => panic!("the admission result should win over a stalled handler"),
    }
}

/// Build a slot whose handler and pending admission never make progress.
fn stalled_stream() -> ActiveStream {
    ActiveStream {
        key: None,
        route_id: 1,
        correlation_id: None,
        sender: None,
        handler: Box::pin(future::pending()),
        pending: Some(PendingSend {
            future: Box::pin(future::pending()),
            is_last: false,
        }),
    }
}