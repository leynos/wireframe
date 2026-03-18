//! Pooled client support built on per-socket `bb8` pools.
//!
//! Each pool slot owns one physical socket managed by `bb8`. Wireframe layers
//! admission permits across those slots so multiple callers can target the
//! same warm socket while transport access remains serialized per socket.

mod client_pool;
mod config;
mod handle;
mod lease;
mod managed;
mod manager;
mod policy;
mod scheduler;
mod slot;

pub use client_pool::WireframeClientPool;
pub use config::ClientPoolConfig;
pub use handle::PoolHandle;
pub use lease::PooledClientLease;
pub use policy::PoolFairnessPolicy;
