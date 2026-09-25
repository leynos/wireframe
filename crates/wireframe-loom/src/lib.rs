//! Loom models of the synchronization Wireframe's connection actor shares
//! with its producers.
//!
//! This package holds no code of its own. Its integration tests, in `tests/`,
//! compile only under `--cfg loom` and run through `make test-loom`; the
//! developers' guide records what they schedule and what they cannot.
