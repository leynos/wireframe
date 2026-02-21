# v0.1.0 to v0.2.0 migration guide

This guide summarizes the breaking changes that must be addressed when
migrating from wireframe v0.1.0 to v0.2.0.

## Configuration builder naming update

The configuration builder method name has been updated to use the
en-GB-oxendict "-ize" spelling.

- Old name: `BackoffConfig::normalised`
- New name: `BackoffConfig::normalized`

Update any references accordingly, including documentation and code examples.

```rust
let config = BackoffConfig::normalized(...);
```

## Cargo feature gate rename

The crate feature used to expose test helper APIs was renamed.

- Old feature: `test-helpers`
- New feature: `test-support`

When a crate enables this feature in `Cargo.toml`, update it as follows:

```toml
# Before
[dependencies]
wireframe = { version = "0.2.0", features = ["test-helpers"] }

# After
[dependencies]
wireframe = { version = "0.2.0", features = ["test-support"] }
```

## Payload accessors

The consuming payload accessors were renamed to follow Rust idioms.

- `PacketParts::payload(self)` has been removed. Use
  `PacketParts::into_payload(self)` instead.
- `FragmentParts::payload(self)` has been removed. Use
  `FragmentParts::into_payload(self)` instead.

```rust
// Before
let packet_payload = packet_parts.payload();
let fragment_payload = fragment_parts.payload();

// After
let packet_payload = packet_parts.into_payload();
let fragment_payload = fragment_parts.into_payload();
```

## Vocabulary normalization by layer

Wireframe now documents one canonical vocabulary per architectural layer in
`docs/users-guide.md` and `docs/developers-guide.md`.

No additional public symbol renames were introduced in this normalization pass.
The migration impact is terminology alignment in docs and rustdoc.

| Prior wording                                                 | Canonical wording   | Scope                                    |
| ------------------------------------------------------------- | ------------------- | ---------------------------------------- |
| frame identifier (routing context)                            | envelope identifier | `Packet`, `Envelope`, `PacketParts` docs |
| frame terminator (`Packet::is_stream_terminator` docs)        | envelope terminator | Packet rustdoc and users' guide examples |
| generic custom packet type named `MyFrame` in packet examples | `MyEnvelope`        | users' guide and rustdoc examples        |

When updating application code, prefer names that follow the same layer model:
`Frame*` for transport codec units, `Envelope` for routing wrappers, and
`Message` for typed domain payloads. Use `packet` as a trait-abstraction alias
where the API explicitly uses `Packet` or `PacketParts`.

## Unified error surface

Wireframe now exposes one canonical crate-level error type:
`wireframe::WireframeError`. The crate-level result alias
`wireframe::Result<T>` also resolves to this type.

- Builder setup errors such as duplicate routes are now reported through
  `wireframe::WireframeError::DuplicateRoute`.
- Streaming and transport errors still use `Io`, `Protocol`, and `Codec`
  variants on the same `wireframe::WireframeError` type.

```rust
// Before
use wireframe::app::WireframeError as AppError;
use wireframe::response::WireframeError as StreamError;

// After
use wireframe::WireframeError;
use wireframe::Result;
```

## Public API surface reorganization

The crate root is now intentionally minimal. Detailed APIs moved to their
owning modules, and root re-exports were removed.

- Root now exposes canonical `wireframe::Result<T>` and
  `wireframe::WireframeError`.
- Use `wireframe::<module>::...` for specialized APIs.
- `wireframe::prelude::*` is available as an optional convenience import for
  high-frequency types.

### Removed root re-exports and their new module paths

- `wireframe::AppDataStore` ->
  `wireframe::app_data_store::AppDataStore`
- `wireframe::BincodeSerializer`, `wireframe::Serializer` ->
  `wireframe::serializer::{BincodeSerializer, Serializer}`
- `wireframe::ConnectionActor` ->
  `wireframe::connection::ConnectionActor`
- `wireframe::CorrelatableFrame` ->
  `wireframe::correlation::CorrelatableFrame`
- `wireframe::ConnectionContext`, `wireframe::ProtocolHooks`,
  `wireframe::WireframeProtocol` ->
  `wireframe::hooks::{ConnectionContext, ProtocolHooks, WireframeProtocol}`
- `wireframe::ClientCodecConfig`, `wireframe::ClientError`,
  `wireframe::ClientProtocolError` -> `wireframe::client::{...}`
- `wireframe::ClientWireframeError`, `wireframe::SocketOptions`,
  `wireframe::WireframeClient` -> `wireframe::client::{...}`
- `wireframe::CodecError`, `wireframe::CodecErrorContext`,
  `wireframe::DefaultRecoveryPolicy`, `wireframe::EofError` ->
  `wireframe::codec::{...}`
- `wireframe::FrameCodec`, `wireframe::FramingError`,
  `wireframe::LengthDelimitedFrameCodec`, `wireframe::MAX_FRAME_LENGTH` ->
  `wireframe::codec::{...}`
- `wireframe::MIN_FRAME_LENGTH`, `wireframe::ProtocolError`,
  `wireframe::RecoveryConfig`, `wireframe::RecoveryPolicy`,
  `wireframe::RecoveryPolicyHook` -> `wireframe::codec::{...}`
- `wireframe::DefaultFragmentAdapter`, `wireframe::FRAGMENT_MAGIC`,
  `wireframe::FragmentAdapter`, `wireframe::FragmentAdapterError` ->
  `wireframe::fragment::{...}`
- `wireframe::FragmentBatch`, `wireframe::FragmentError`,
  `wireframe::FragmentFrame`, `wireframe::FragmentHeader`,
  `wireframe::FragmentIndex`, `wireframe::FragmentSeries` ->
  `wireframe::fragment::{...}`
- `wireframe::FragmentStatus`, `wireframe::FragmentationConfig`,
  `wireframe::FragmentationError`, `wireframe::Fragmenter`,
  `wireframe::MessageId`, `wireframe::ReassembledMessage` ->
  `wireframe::fragment::{...}`
- `wireframe::Reassembler`, `wireframe::ReassemblyError`,
  `wireframe::decode_fragment_payload`, `wireframe::encode_fragment_payload`,
  `wireframe::fragment_overhead` -> `wireframe::fragment::{...}`
- `wireframe::AssembledMessage`, `wireframe::ContinuationFrameHeader`,
  `wireframe::FirstFrameHeader`, `wireframe::FirstFrameInput` ->
  `wireframe::message_assembler::{...}`
- `wireframe::FirstFrameInputError`, `wireframe::FrameHeader`,
  `wireframe::FrameSequence`, `wireframe::MessageAssembler`,
  `wireframe::MessageAssemblyError` -> `wireframe::message_assembler::{...}`
- `wireframe::MessageAssemblyState`, `wireframe::MessageKey`,
  `wireframe::MessageSeries`, `wireframe::MessageSeriesError`,
  `wireframe::MessageSeriesStatus`, `wireframe::ParsedFrameHeader` ->
  `wireframe::message_assembler::{...}`
- `wireframe::CODEC_ERRORS`, `wireframe::CONNECTIONS_ACTIVE`,
  `wireframe::Direction`, `wireframe::ERRORS_TOTAL`,
  `wireframe::FRAMES_PROCESSED` -> `wireframe::metrics::{...}`
- `wireframe::DEFAULT_BODY_CHANNEL_CAPACITY`,
  `wireframe::RequestBodyReader`, `wireframe::RequestBodyStream`,
  `wireframe::RequestParts`, `wireframe::body_channel` ->
  `wireframe::request::{...}`
- `wireframe::FrameStream`, `wireframe::Response` ->
  `wireframe::response::{FrameStream, Response}`
- `wireframe::ConnectionId`, `wireframe::SessionRegistry` ->
  `wireframe::session::{ConnectionId, SessionRegistry}`

Example migration:

```rust
// Before
use wireframe::{Response, Serializer, WireframeClient};

// After
use wireframe::{
    client::WireframeClient,
    response::Response,
    serializer::Serializer,
};
```

## Test support visibility changes

Connection actor test helpers are no longer reachable in normal builds.

- `wireframe::connection::test_support` now requires
  `cfg(any(test, feature = "test-support"))` in addition to `cfg(not(loom))`.
- Production consumers should not depend on `connection::test_support`.
- Internal and integration test suites can continue to use the `test-support`
  feature where required.
