//! Registry of active connection push handles.
//!
//! `SessionRegistry` stores non-owning weak references to [`PushHandle`]s,
//! allowing asynchronous tasks to send frames to live connections without
//! preventing their cleanup. Dead entries can be pruned opportunistically or
//! lazily at lookup time.
use std::sync::Weak;

use dashmap::DashMap;

use crate::push::{FrameLike, PushHandle, PushHandleInner};

/// Identifier assigned to a connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ConnectionId(u64);

impl From<u64> for ConnectionId {
    fn from(value: u64) -> Self { Self(value) }
}

/// Concurrent registry of push handles keyed by [`ConnectionId`].
#[derive(Default)]
pub struct SessionRegistry<F>(DashMap<ConnectionId, Weak<PushHandleInner<F>>>);

impl<F: FrameLike> SessionRegistry<F> {
    /// Retrieve a `PushHandle` for `id` if the connection is still alive.
    pub fn get(&self, id: &ConnectionId) -> Option<PushHandle<F>> {
        self.0
            .get(id)
            .and_then(|weak| weak.upgrade())
            .map(PushHandle::from_arc)
    }

    /// Insert a handle for a newly established connection.
    pub fn insert(&self, id: ConnectionId, handle: &PushHandle<F>) {
        self.0.insert(id, handle.downgrade());
    }

    /// Remove a handle, typically on connection teardown.
    pub fn remove(&self, id: &ConnectionId) { self.0.remove(id); }

    /// Drop entries whose connections have terminated.
    pub fn prune(&self) { self.0.retain(|_, weak| weak.strong_count() > 0); }
}
