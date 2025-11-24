# Changelog

All notable changes to this project will be documented in this file.

## Unreleased

- Deprecated `SharedState::new` (since 0.2.0); construct via `inner.into()`
  instead.
- Breaking: Marked `ServerError` as `#[non_exhaustive]`. Downstream consumers
  must add a wildcard arm when matching it.
- Exposed `MAX_PUSH_RATE` for configuring push queue rate limits.
- Added a `Fragmenter` helper that slices oversized messages into sequential
  fragments, stamping each piece with a `FragmentHeader` for transparent
  transport-level reassembly.
