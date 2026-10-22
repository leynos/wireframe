//! Public client-pool surface that must compile with `test-support` disabled.
//!
//! This fixture is the control half of the feature-gate check. It uses only
//! stable public API, so a compiler error here means the check itself is broken
//! rather than that the accessors are gated correctly. `make test-feature-gate`
//! rejects an unexpected failure instead of reading it as a passing gate.

use std::net::SocketAddr;

use wireframe::client::{ClientError, ClientPoolConfig, WireframeClient};

/// Build a pool, take a lease, and read the public configuration back.
///
/// Deliberately avoids `pool_size` and `slot_id`, which are
/// `test-support`-only; see `support_accessors.rs` for those.
///
/// # Errors
///
/// Returns the pool's own error if connecting or acquiring a lease fails.
pub async fn use_public_pool_api(addr: SocketAddr) -> Result<usize, ClientError> {
    let config = ClientPoolConfig::default().pool_size(2);
    let pool = WireframeClient::builder()
        .connect_pool(addr, config)
        .await?;
    let _lease = pool.acquire().await?;

    Ok(config.pool_size_value())
}
