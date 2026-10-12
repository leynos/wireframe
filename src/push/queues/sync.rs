//! Synchronization aliases shared by the push queue's inner state.
//!
//! The Loom models in `crates/wireframe-loom` drive this state directly, and
//! Loom can only schedule interleavings of its own instrumented primitives.
//! Under `--cfg loom` the mutex and the atomics below therefore resolve to
//! Loom's, so a model exercising the dead-letter drop accounting interleaves
//! those accesses rather than treating them as opaque.
//!
//! `Arc` and `Weak` stay on the standard library in both builds: they carry no
//! scheduling behaviour a model needs to explore, and Loom offers no
//! replacement for them.

#[cfg(not(loom))]
pub(super) use std::sync::{
    Arc,
    Mutex,
    Weak,
    atomic::{AtomicUsize, Ordering},
};
#[cfg(loom)]
pub(super) use std::sync::{Arc, Weak};

#[cfg(loom)]
pub(super) use loom::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};
