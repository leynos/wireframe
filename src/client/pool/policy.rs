//! Public fairness policies for pooled client handles.

/// Fairness policy used when many `PoolHandle`s contend for pooled leases.
///
/// `RoundRobin` is the default because it gives stable logical sessions turns
/// in rotation. `Fifo` preserves the exact order in which blocked acquisitions
/// arrived.
///
/// # Examples
///
/// ```
/// use wireframe::client::{ClientPoolConfig, PoolFairnessPolicy};
///
/// let config = ClientPoolConfig::default()
///     .pool_size(2)
///     .fairness_policy(PoolFairnessPolicy::RoundRobin);
///
/// assert_eq!(
///     config.fairness_policy_value(),
///     PoolFairnessPolicy::RoundRobin
/// );
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PoolFairnessPolicy {
    /// Rotate grants across stable logical-session handles.
    #[default]
    RoundRobin,
    /// Serve waiting handles in the order they blocked.
    Fifo,
}
