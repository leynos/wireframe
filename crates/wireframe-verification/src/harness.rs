//! Shared Stateright checker helpers for bounded repository validation.

use std::{fmt::Debug, hash::Hash, num::NonZeroUsize};

use stateright::{Checker, CheckerBuilder, Model};

/// Default maximum depth for repository-local Stateright checks.
pub const DEFAULT_TARGET_MAX_DEPTH: NonZeroUsize = match NonZeroUsize::new(8) {
    Some(v) => v,
    _ => unreachable!(),
};

/// Default state budget for repository-local Stateright checks.
pub const DEFAULT_TARGET_STATE_COUNT: NonZeroUsize = match NonZeroUsize::new(5_000) {
    Some(v) => v,
    _ => unreachable!(),
};

/// Bounds applied to a Stateright checker run.
///
/// Zero is rejected at construction time because Stateright interprets
/// `0` as "unbounded" (converting it to `None` via `NonZeroUsize::new`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VerificationBounds {
    /// Maximum search depth for the checker.
    pub max_depth: NonZeroUsize,
    /// Maximum state budget for the checker.
    pub max_state_count: NonZeroUsize,
}

impl Default for VerificationBounds {
    /// Returns bounds using [`DEFAULT_TARGET_MAX_DEPTH`] and [`DEFAULT_TARGET_STATE_COUNT`].
    fn default() -> Self {
        Self {
            max_depth: DEFAULT_TARGET_MAX_DEPTH,
            max_state_count: DEFAULT_TARGET_STATE_COUNT,
        }
    }
}

/// Build a repository-standard bounded checker for a model.
///
/// # Example
///
/// ```rust
/// use wireframe_verification::{
///     connection_model::PlaceholderConnectionModel,
///     harness::{VerificationBounds, checker},
/// };
///
/// let _builder = checker(
///     PlaceholderConnectionModel::default(),
///     VerificationBounds::default(),
/// );
/// ```
pub fn checker<M>(model: M, bounds: VerificationBounds) -> CheckerBuilder<M>
where
    M: Model + Send + Sync + 'static,
    M::State: Hash + Send + Sync,
{
    model
        .checker()
        .target_max_depth(bounds.max_depth.get())
        .target_state_count(bounds.max_state_count.get())
}

/// Run the repository-standard bounded checker and assert every property.
pub fn assert_model_properties<M>(model: M)
where
    M: Model + Send + Sync + 'static,
    M::Action: Debug + Clone + PartialEq,
    M::State: Debug + Clone + Hash + PartialEq + Send + Sync + 'static,
{
    assert_model_properties_with_bounds(model, VerificationBounds::default());
}

/// Run a bounded checker with explicit limits and assert every property.
pub fn assert_model_properties_with_bounds<M>(model: M, bounds: VerificationBounds)
where
    M: Model + Send + Sync + 'static,
    M::Action: Debug + Clone + PartialEq,
    M::State: Debug + Clone + Hash + PartialEq + Send + Sync + 'static,
{
    let checker = checker(model, bounds).spawn_bfs().join();
    checker.assert_properties();
}
