# Changelog

All notable changes to this project will be documented in this file.

## Unreleased

- Deprecated `SharedState::new` (since 0.2.0); construct via `inner.into()`
  instead.
- Breaking: Marked `ServerError` as `#[non_exhaustive]`. Downstream consumers
  must add a wildcard arm when matching it.
- Breaking: Renamed `BackoffConfig::normalised` to `BackoffConfig::normalized`
  to align the public API spelling with American English.
- Exposed `MAX_PUSH_RATE` for configuring push queue rate limits.
- Added a `Fragmenter` helper that slices oversized messages into sequential
  fragments, stamping each piece with a `FragmentHeader` for transparent
  transport-level reassembly.
- Breaking: Changed `FragmentError::IndexOverflow` and
  `FragmentationError::IndexOverflow` from unit variants to struct variants
  carrying a `last: FragmentIndex` field. This field records the final valid
  index observed before the counter would overflow `u32::MAX`.

  **Migration guide:**

  Pattern matches against the old unit variant must be updated to destructure
  or wildcard the new field:

  ```rust
  // Before (0.1.x): unit variant
  match err {
      FragmentError::IndexOverflow => { /* ... */ }
      // ...
  }

  // After (0.2+): struct variant with `last` field
  match err {
      FragmentError::IndexOverflow { last } => {
          eprintln!("overflow after fragment index {last}");
      }
      // ...
  }
  ```

  The same change applies to `FragmentationError::IndexOverflow`:

  ```rust
  // Before (0.1.x)
  Err(FragmentationError::IndexOverflow) => { /* ... */ }

  // After (0.2+)
  Err(FragmentationError::IndexOverflow { last }) => {
      log::warn!("cannot fragment: index overflow after {last}");
  }
  ```

  If the `last` value is not needed, use `{ .. }` to ignore it:

  ```rust
  match err {
      FragmentError::IndexOverflow { .. } => { /* handle overflow */ }
      // ...
  }
  ```
