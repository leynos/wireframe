//! Unit tests for [`SendStreamingConfig`] accessors.

use std::time::Duration;

use crate::client::SendStreamingConfig;

/// The `chunk_size` getter has no production callers (internal code reads the
/// field directly), so its accessor mutants survive unless asserted directly.
/// The neighbouring `timeout` getter is guarded the same way.
#[test]
fn config_accessors_reflect_builder() {
    let default = SendStreamingConfig::default();
    assert_eq!(default.chunk_size(), None, "default chunk size is unset");
    assert_eq!(default.timeout(), None, "default timeout is unset");

    let configured = SendStreamingConfig::default()
        .with_chunk_size(4096)
        .with_timeout(Duration::from_secs(3));
    assert_eq!(configured.chunk_size(), Some(4096));
    assert_eq!(configured.timeout(), Some(Duration::from_secs(3)));
}
