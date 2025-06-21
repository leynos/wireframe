# Testing Helpers for `wireframe`

`wireframe-testing` is a proposed companion crate providing utilities for unit
and integration tests. It focuses on driving `WireframeApp` instances with raw
frames, enabling fast tests without opening real network connections.

## Motivation

The existing tests in [`tests/`](../tests) use helper functions such as
`run_app_with_frame` and `run_app_with_frames` to feed length‑prefixed frames
through an in‑memory duplex stream. These helpers simplify testing handlers by
allowing assertions on encoded responses without spinning up a full server.
Encapsulating this logic in a dedicated crate keeps test code concise and
reusable across projects.

## Crate Layout

- `wireframe-testing`
  - `Cargo.toml` enabling the `tokio` and `rstest` dependencies used by the
    helpers.
  - `src/lib.rs` exposing asynchronous functions for driving apps with raw
    frames.

The crate would live in a `wireframe-testing/` directory alongside the main
`wireframe` crate.

## Proposed API

```rust
use tokio::io::Result as IoResult;
use wireframe::app::WireframeApp;

/// Feed a single frame into `app` using an in-memory duplex stream.
async fn drive_with_frame(app: WireframeApp, frame: Vec<u8>) -> IoResult<Vec<u8>>;

/// Drive `app` with multiple frames, returning all bytes written by the app.
async fn drive_with_frames(app: WireframeApp, frames: Vec<Vec<u8>>) -> IoResult<Vec<u8>>;
```

These functions mirror the behaviour of `run_app_with_frame` and
`run_app_with_frames` found in the repository’s test utilities. They create a
`tokio::io::duplex` stream, spawn the application as a background task, and
write the provided frame(s) to the client side of the stream. After the app
finishes processing, the helpers collect the bytes written back and return them
for inspection.

### Custom Buffer Capacity

A variant accepting a buffer `capacity` allows fine‑tuning the size of the
in‑memory duplex channel, matching the existing
`run_app_with_frame_with_capacity` and `run_app_with_frames_with_capacity`
helpers.

## Example Usage

```rust
#[tokio::test]
async fn handler_echoes_message() {
    let app = WireframeApp::new()
        .unwrap()
        .frame_processor(LengthPrefixedProcessor::default())
        .route(1, Arc::new(|_| Box::pin(async {})))
        .unwrap();

    let frame = build_test_frame();
    let out = drive_with_frame(app, frame).await.unwrap();
    assert_eq!(out, expected_bytes());
}
```

This pattern mirrors the style of `tests/routes.rs`, where handlers are invoked
with prebuilt frames and their responses decoded for assertions.

## Benefits

- **Isolation**: Handlers can be tested without spinning up a full server or
  opening sockets.
- **Reusability**: Projects consuming `wireframe` can depend on
  `wireframe-testing` in their dev‑dependencies to leverage the same helpers.
- **Clarity**: Abstracting the duplex stream logic keeps test cases focused on
  behaviour instead of transport details.

## Next Steps

Implement the crate in a new directory, export the helper functions, and migrate
existing tests to use them. Additional fixtures (e.g., prebuilt frame
processors) can be added over time as test coverage grows.
