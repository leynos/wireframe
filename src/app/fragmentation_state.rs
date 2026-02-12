//! Connection-scoped fragmentation type aliases used by app frame handling.

/// Concrete adapter state used by `WireframeApp` connection processing.
pub(crate) type FragmentationState = crate::fragment::DefaultFragmentAdapter;

/// Decode and reassembly errors surfaced by adapter processing.
pub(crate) type FragmentProcessError = crate::fragment::FragmentAdapterError;
