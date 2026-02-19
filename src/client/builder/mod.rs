//! Builder for configuring and connecting a wireframe client.

/// Reconstructs `WireframeClientBuilder` with one field updated to a new value.
///
/// This macro reduces duplication in type-changing builder methods that need to
/// create a new builder instance with different generic parameters. When a type
/// parameter changes, struct update syntax (`..self`) cannot be used, so fields
/// must be copied explicitly.
///
/// Use this macro for small builders with a limited number of fields (five in
/// this case) and single-field updates per method. For larger builders with
/// many coordinated updates, prefer a dedicated helper method to keep the
/// reconstruction logic centralized and easier to audit (see
/// `WireframeApp::rebuild_with_params` and
/// `docs/builder-pattern-conventions.md`).
///
/// The `lifecycle_hooks` field requires special handling because `LifecycleHooks<C>`
/// is parameterized by the connection state type. When changing `S` or `P`, the
/// hooks can be moved directly since `C` is unchanged. When changing `C` via
/// `on_connection_setup`, a new `LifecycleHooks<C2>` must be constructed.
macro_rules! builder_field_update {
    // Serializer change: preserves P and C, moves lifecycle_hooks unchanged
    ($self:expr,serializer = $value:expr) => {
        WireframeClientBuilder {
            serializer: $value,
            codec_config: $self.codec_config,
            socket_options: $self.socket_options,
            preamble_config: $self.preamble_config,
            lifecycle_hooks: $self.lifecycle_hooks,
        }
    };
    // Preamble change: preserves S and C, moves lifecycle_hooks unchanged
    ($self:expr,preamble_config = $value:expr) => {
        WireframeClientBuilder {
            serializer: $self.serializer,
            codec_config: $self.codec_config,
            socket_options: $self.socket_options,
            preamble_config: $value,
            lifecycle_hooks: $self.lifecycle_hooks,
        }
    };
    // Lifecycle hooks change: preserves S and P, changes C
    ($self:expr,lifecycle_hooks = $value:expr) => {
        WireframeClientBuilder {
            serializer: $self.serializer,
            codec_config: $self.codec_config,
            socket_options: $self.socket_options,
            preamble_config: $self.preamble_config,
            lifecycle_hooks: $value,
        }
    };
}

mod codec;
mod connect;
mod core;
mod lifecycle;
mod preamble;
mod serializer;

pub use core::WireframeClientBuilder;
