//! Negative fixture: the `test-support` accessors must not exist without it.
//!
//! This binary is compiled only when the `test-support` feature is *disabled*,
//! and it must fail. The check script requires the errors to name
//! `pool_size` and `slot_id`; any other failure is reported as a broken probe,
//! not as a passing gate.

use std::{net::SocketAddr, process::ExitCode};

use wireframe::client::{ClientError, ClientPoolConfig, WireframeClient};

/// Exactly the calls that `test-support` is supposed to enable.
async fn read_absent_accessors(addr: SocketAddr) -> Result<usize, ClientError> {
    let pool = WireframeClient::builder()
        .connect_pool(addr, ClientPoolConfig::default().pool_size(2))
        .await?;
    let lease = pool.acquire().await?;

    Ok(pool.pool_size() + lease.slot_id())
}

fn main() -> ExitCode {
    // Referencing the function keeps it reachable; the point of the probe is
    // that its body does not compile, which the check script asserts on.
    let _ = read_absent_accessors as fn(SocketAddr) -> _;
    ExitCode::SUCCESS
}
