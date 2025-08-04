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

impl ConnectionId {
    /// Create a new [`ConnectionId`] with the provided value.
    #[must_use]
    pub fn new(id: u64) -> Self { Self(id) }

    /// Return the inner `u64` representation.
    #[must_use]
    pub fn as_u64(&self) -> u64 { self.0 }
}

impl std::fmt::Display for ConnectionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ConnectionId({})", self.0)
    }
}

/// Concurrent registry of push handles keyed by [`ConnectionId`].
#[derive(Default)]
pub struct SessionRegistry<F>(DashMap<ConnectionId, Weak<PushHandleInner<F>>>);

impl<F: FrameLike> SessionRegistry<F> {
    /// Retrieve a `PushHandle` for `id` if the connection is still alive.
    pub fn get(&self, id: &ConnectionId) -> Option<PushHandle<F>> {
        let guard = self.0.get(id);
        let handle = guard.as_ref().and_then(|weak| weak.upgrade());
        drop(guard);
        if handle.is_none() {
            self.0.remove_if(id, |_, weak| weak.strong_count() == 0);
        }
        handle.map(PushHandle::from_arc)
    }

    /// Insert a handle for a newly established connection.
    pub fn insert(&self, id: ConnectionId, handle: &PushHandle<F>) {
        self.0.insert(id, handle.downgrade());
    }

    /// Remove a handle, typically on connection teardown.
    pub fn remove(&self, id: &ConnectionId) { self.0.remove(id); }

    /// Remove all stale weak references without returning any handles.
    ///
    /// `DashMap::retain` acquires per-bucket write locks, so other operations
    /// may contend briefly while the registry is pruned.
    pub fn prune(&self) { self.0.retain(|_, weak| weak.strong_count() > 0); }

    /// Prune stale weak references, then collect the remaining live handles.
    ///
    /// This holds per-bucket write locks while iterating. Use [`prune`] from a
    /// maintenance task when only cleanup is required.
    #[must_use]
    pub fn active_handles(&self) -> Vec<(ConnectionId, PushHandle<F>)> {
        let mut handles = Vec::with_capacity(self.0.len());
        self.0.retain(|id, weak| {
            if let Some(inner) = weak.upgrade() {
                handles.push((*id, PushHandle::from_arc(inner)));
                true
            } else {
                false
            }
        });
        handles
    }

    /// Prune stale weak references, then return the IDs of the live connections.
    ///
    /// This holds per-bucket write locks while iterating. Use [`prune`] from a
    /// maintenance task when only cleanup is required.
    #[must_use]
    pub fn active_ids(&self) -> Vec<ConnectionId> {
        let mut ids = Vec::with_capacity(self.0.len());
        self.0.retain(|id, weak| {
            if weak.strong_count() > 0 {
                ids.push(*id);
                true
            } else {
                false
            }
        });
        ids
    }
}
