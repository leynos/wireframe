# Changelog

All notable changes to this project will be documented in this file.

## Unreleased

- Deprecated `SharedState::new` (since 0.2.0); construct via `inner.into()`
  instead.
- Marked `ServerError` as `#[non_exhaustive]`. Downstream consumers must add a
  wildcard arm when matching on it.
