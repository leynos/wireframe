# wireframe_testing

Helper utilities for exercising [`wireframe`](https://crates.io/crates/wireframe)
applications in tests without opening real sockets. The crate runs a
`WireframeApp` against in-memory duplex streams, captures every frame the app
emits, and provides small helpers for encoding or decoding frames so assertions
stay focused on behaviour rather than plumbing.

- Drive an app with length-delimited frames or bincode-serialised payloads.
- Collect multi-frame responses into a single buffer for snapshot-style
  assertions.
- Capture log output during tests with a lightweight logger guard.

## Quick start

```rust
use wireframe::app::WireframeApp;
use wireframe_testing::{decode_frames, drive_with_bincode, logger};

#[tokio::test]
async fn drives_app() -> std::io::Result<()> {
    let _log_guard = logger();
    let app = WireframeApp::new().expect("failed to initialise app");

    let raw = drive_with_bincode(app, 42u8).await?;
    let frames = decode_frames(raw);

    assert_eq!(frames.len(), 1);
    Ok(())
}
```

## Running the tests

- `cargo test -p wireframe_testing`

The command exercises the helpers in isolation. Run it before publishing
changes to ensure the test fixtures continue to match the main `wireframe`
crate's expectations.
