//! The `test-support`-only pool accessors.
//!
//! Compiled only into the `test-support` lib target, where the accessors must
//! exist. The matching `bin` target declares these two calls as its entire body
//! and must fail to compile without the feature; `scripts/check-feature-gate.sh`
//! asserts that failure names the missing methods.

use std::net::SocketAddr;

use wireframe::client::{ClientError, ClientPoolConfig, WireframeClient};

/// Read the pool's slot count and the lease's selected slot.
///
/// # Errors
///
/// Returns the pool's own error if connecting or acquiring a lease fails.
pub async fn read_support_accessors(addr: SocketAddr) -> Result<usize, ClientError> {
    let pool = WireframeClient::builder()
        .connect_pool(addr, ClientPoolConfig::default().pool_size(2))
        .await?;
    let lease = pool.acquire().await?;

    Ok(pool.pool_size() + lease.slot_id())
}
