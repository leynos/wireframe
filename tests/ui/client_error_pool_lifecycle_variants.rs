//! Downstream callers can match the pool lifecycle errors and inspect them.

use wireframe::client::ClientError;

fn is_recoverable(error: ClientError) -> bool {
    match error {
        ClientError::PoolClosed => false,
        ClientError::SchedulerFailed => false,
        _ => true,
    }
}

fn main() {
    let _ = is_recoverable as fn(ClientError) -> bool;
    assert!(!ClientError::PoolClosed.should_recycle_connection(), "assertion failed: !ClientError::PoolClosed.should_recycle_connection()");
    assert!(!ClientError::SchedulerFailed.should_recycle_connection(), "assertion failed: !ClientError::SchedulerFailed.should_recycle_connection()");
}
