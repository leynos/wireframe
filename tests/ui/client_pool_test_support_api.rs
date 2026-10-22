//! Check the `test-support` pool accessors stay exported and callable.
//!
//! Both accessors are compile-time gated, so a broken `cfg` would only show
//! up for downstream test crates. This fixture is the compile check.

use std::net::SocketAddr;

use wireframe::client::{ClientError, ClientPoolConfig, WireframeClient};

async fn inspect_pool(addr: SocketAddr) -> Result<usize, ClientError> {
    let pool = WireframeClient::builder()
        .connect_pool(addr, ClientPoolConfig::default().pool_size(2))
        .await?;
    let lease = pool.acquire().await?;

    Ok(pool.pool_size() + lease.slot_id())
}

fn main() { let _ = inspect_pool; }
