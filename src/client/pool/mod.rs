//! Pooled client support built on per-socket `bb8` pools.
//!
//! Each pool slot owns one physical socket managed by `bb8`. Wireframe layers
//! admission permits across those slots so multiple callers can target the
//! same warm socket while transport access remains serialized per socket.

mod config;
mod lease;
mod managed;
mod manager;
mod client_pool;
mod slot;

pub use config::ClientPoolConfig;
pub use client_pool::WireframeClientPool;
pub use lease::PooledClientLease;
