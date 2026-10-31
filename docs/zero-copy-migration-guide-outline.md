# Zero-copy migration guide outline

This outline lists the public signatures that the zero-copy migration changes
and sketches the before-and-after examples the final migration guide will
carry. It closes roadmap item `1.2.3` of the
[zero-copy migration roadmap](zero-copy-frame-and-payload-migration-roadmap.md)
and roadmap item `10.2.3` of the [project roadmap](roadmap.md).

It is an outline, not the guide. Every section names the roadmap item that must
land before the corresponding text can be published as guidance, and every
example that depends on an unimplemented type is labelled **Proposed**.

## How to read this document

The migration is governed by three accepted decisions and one unaccepted
proposal. Confusing the two would put unapproved policy into release notes, so
this document separates them before anything else.

| Source                                                                                                | Status                     | What it fixes                                               |
| ----------------------------------------------------------------------------------------------------- | -------------------------- | ----------------------------------------------------------- |
| [ADR 008](adr-008-zero-copy-public-byte-container.md)                                                 | Accepted                   | Stable byte container plus explicit edit-on-demand mutation |
| [ADR 009](adr-009-vec-u8-migration-rollout.md)                                                        | Accepted                   | Finite compatibility helpers, deprecation, removal timing   |
| [ADR 010](adr-010-transport-frame-boundary-for-zero-copy.md)                                          | Accepted                   | Actor stays envelope-oriented; codec driver owns frames     |
| Serializer amendment in ADRs 008 and 009; [#538](https://github.com/leynos/wireframe/issues/538) plan | **Proposed, not accepted** | Serializer writer output and `PayloadBytes` placement       |

The proposed amendment changes three things the accepted text already settles:
whether a deprecated `serialize_to_vec` shim ships at all, whether the
serializer returns the wrapper directly, and which release removes the helpers.
Release guidance must describe the accepted ADR 009 shape until the amendment
is accepted through roadmap item `11.1.2.1`. The amendment's status is also
recorded in the [release plan](zero-copy-release-plan.md).

A second caveat applies to every example below: `PayloadBytes` does not exist
in the source tree. `src/payload.rs` is absent, and the serializer, hooks, and
middleware still use `Vec<u8>`. The type is specified by ADR 008 and by the
[#538 plan](execplans/issue-538-plan-zero-copy-serializer-output-migration.md),
but no code implements it. Examples that use it are therefore labelled
**Proposed** without exception.

## Affected public signatures

Line references are to the current tree. The "planned" column states the
direction the accepted ADRs require; it does not promise a literal signature,
because the exact shape is fixed by the implementing roadmap item.

### Packet surfaces

| Item                | Current signature                                                                  | Location                  | Planned change                                                      |
| ------------------- | ---------------------------------------------------------------------------------- | ------------------------- | ------------------------------------------------------------------- |
| `PacketParts` field | `payload: Vec<u8>`                                                                 | `src/app/envelope.rs:143` | Becomes the stable byte wrapper (roadmap `3.1.1`)                   |
| `PacketParts::new`  | `pub const fn new(id: u32, correlation_id: Option<u64>, payload: Vec<u8>) -> Self` | `src/app/envelope.rs:203` | Takes the stable wrapper; `Vec<u8>` callers use the helper          |
| `PacketParts` read  | `pub fn payload_bytes(&self) -> &[u8]`                                             | `src/app/envelope.rs:180` | Unchanged; borrow stays a slice                                     |
| `PacketParts` take  | `pub fn into_payload(self) -> Vec<u8>`                                             | `src/app/envelope.rs:261` | Returns the stable wrapper; a vector-taking shim is the same helper |
| `Envelope` field    | `payload: Vec<u8>`                                                                 | `src/app/envelope.rs:157` | Becomes the stable byte wrapper (roadmap `3.1.1`)                   |
| `Envelope::new`     | `pub const fn new(id: u32, correlation_id: Option<u64>, payload: Vec<u8>) -> Self` | `src/app/envelope.rs:161` | As `PacketParts::new`                                               |
| `Envelope` read     | `pub fn payload_bytes(&self) -> &[u8]`                                             | `src/app/envelope.rs:248` | Unchanged                                                           |
| `Packet` trait      | `fn into_parts(self) -> PacketParts`, `fn from_parts(parts: PacketParts) -> Self`  | `src/app/envelope.rs:62`  | Signatures unchanged; the payload type inside `PacketParts` changes |

`Envelope` also derives `bincode::Decode` and `bincode::Encode`
(`src/app/envelope.rs:148`). Moving its payload off `Vec<u8>` therefore forces
a decision the ADRs do not make: either the wrapper implements bincode's
traits, or the derive is replaced by a manual implementation. The final guide
must state which, and roadmap item `3.1.1` must answer it before the guide is
published. `PacketParts` has no such derive, so it is the easier of the two.

### Middleware surfaces

| Item                          | Current signature                                                 | Location                | Planned change                                          |
| ----------------------------- | ----------------------------------------------------------------- | ----------------------- | ------------------------------------------------------- |
| `ServiceRequest::new`         | `pub fn new(frame: Vec<u8>, correlation_id: Option<u64>) -> Self` | `src/middleware.rs:50`  | Takes the stable wrapper                                |
| `ServiceRequest::frame`       | `pub fn frame(&self) -> &[u8]`                                    | `src/middleware.rs:59`  | Unchanged; read-only borrow stays zero-copy             |
| `ServiceRequest::frame_mut`   | `pub fn frame_mut(&mut self) -> &mut Vec<u8>`                     | `src/middleware.rs:74`  | **Removed**; replaced by the edit-on-demand operation   |
| `ServiceRequest::into_inner`  | `pub fn into_inner(self) -> Vec<u8>`                              | `src/middleware.rs:78`  | Removed or returns the wrapper; see the ambiguity below |
| `ServiceResponse::new`        | `pub fn new(frame: Vec<u8>, correlation_id: Option<u64>) -> Self` | `src/middleware.rs:93`  | As the request counterpart                              |
| `ServiceResponse::frame`      | `pub fn frame(&self) -> &[u8]`                                    | `src/middleware.rs:102` | Unchanged                                               |
| `ServiceResponse::frame_mut`  | `pub fn frame_mut(&mut self) -> &mut Vec<u8>`                     | `src/middleware.rs:106` | **Removed**                                             |
| `ServiceResponse::into_inner` | `pub fn into_inner(self) -> Vec<u8>`                              | `src/middleware.rs:121` | As the request counterpart                              |

ADR 009 is explicit that middleware `&mut Vec<u8>` editors are removed rather
than deprecated. Middleware authors therefore get a hard break in the payload
editing surface, which is why roadmap item `12.1.2` owns the replacement
ergonomics.

### Client hook surfaces

| Item                          | Current signature                                                                                 | Location                                 | Planned change                                           |
| ----------------------------- | ------------------------------------------------------------------------------------------------- | ---------------------------------------- | -------------------------------------------------------- |
| `BeforeSendHook`              | `pub type BeforeSendHook = Arc<dyn Fn(&mut Vec<u8>) + Send + Sync>`                               | `src/client/hooks.rs:148`                | Editor surface over the stable wrapper (roadmap `3.2.1`) |
| `AfterReceiveHook`            | `pub type AfterReceiveHook = Arc<dyn Fn(&mut BytesMut) + Send + Sync>`                            | `src/client/hooks.rs:172`                | Unchanged; inbound already uses `BytesMut`               |
| `RequestHooks::before_send`   | `pub fn before_send<F>(mut self, f: F) -> Self where F: Fn(&mut Vec<u8>) + Send + Sync + 'static` | `src/client/builder/request_hooks.rs:37` | Accepts the new editor; keeps a vector-taking adapter    |
| `RequestHooks::after_receive` | `pub fn after_receive<F>(mut self, f: F) -> Self` (bounded on `Fn(&mut BytesMut)`)                | `src/client/builder/request_hooks.rs:71` | Unchanged                                                |

The adapter ADR 009 promises is `BeforeSendHook::from_vec_fn`, with a
free-function equivalent `before_send_from_vec_fn` on `RequestHooks`. Those
names are accepted policy; their availability is not, because roadmap item
`3.2.1` has not landed.

### Serializer surfaces

| Item                                     | Current signature                                                                                                           | Location                | Planned change                                              |
| ---------------------------------------- | --------------------------------------------------------------------------------------------------------------------------- | ----------------------- | ----------------------------------------------------------- |
| `Serializer::serialize`                  | `fn serialize<M>(&self, value: &M) -> Result<Vec<u8>, Box<dyn Error + Send + Sync>> where M: EncodeWith<Self>, Self: Sized` | `src/serializer.rs:67`  | Returns the stable wrapper (roadmap `3.2.2`)                |
| `SerdeSerializerBridge::serialize_serde` | `fn serialize_serde<T: serde::Serialize>(...) -> Result<Vec<u8>, Box<dyn Error + Send + Sync>>`                             | `src/serializer.rs:33`  | Returns the stable wrapper                                  |
| `BincodeSerializer::serialize`           | `fn serialize<M>(&self, value: &M) -> Result<Vec<u8>, ...>`                                                                 | `src/serializer.rs:122` | Writes directly into the wrapper's buffer                   |
| `EncodeWith::encode_with`                | `fn encode_with(&self, serializer: &S) -> Result<Vec<u8>, Box<dyn Error + Send + Sync>>`                                    | `src/message.rs:68`     | Returns the stable wrapper                                  |
| `Message::to_bytes`                      | `fn to_bytes(&self) -> Result<Vec<u8>, EncodeError>`                                                                        | `src/message.rs:97`     | Retained; the bounded native path is not the transport path |

The serializer row is the one blocked by the unaccepted amendment. ADR 009
already accepts that `serialize` returns the stable wrapper; what is unresolved
is whether `serialize_to_vec` ships as a deprecated shim and which release
removes it. That question is tracked under roadmap item `11.1.2.1`.

### Custom codec surfaces

| Item                              | Current signature                                                       | Location               | Planned change                                               |
| --------------------------------- | ----------------------------------------------------------------------- | ---------------------- | ------------------------------------------------------------ |
| `FrameCodec::frame_payload`       | `fn frame_payload(frame: &Self::Frame) -> &[u8]`                        | `src/codec/mod.rs:79`  | Unchanged                                                    |
| `FrameCodec::frame_payload_bytes` | `fn frame_payload_bytes(frame: &Self::Frame) -> Bytes` (default copies) | `src/codec/mod.rs:90`  | Override to return a cheap clone when the frame stores bytes |
| `FrameCodec::wrap_payload`        | `fn wrap_payload(&self, payload: Bytes) -> Self::Frame`                 | `src/codec/mod.rs:95`  | Unchanged; the argument is already `Bytes`                   |
| `LengthDelimitedFrameCodec`       | `type Frame = Bytes`; `wrap_payload` returns its argument               | `src/codec/mod.rs:272` | Already zero-copy; serves as the reference example           |

Custom codecs are the least disruptive surface. The trait already takes and
returns `Bytes`, so a codec that stores `Bytes` in its frame needs no signature
change; it needs only to override `frame_payload_bytes`, whose default
implementation copies.

### Frame-implementation surfaces

| Item                            | Location                                          | Status                                                                              |
| ------------------------------- | ------------------------------------------------- | ----------------------------------------------------------------------------------- |
| `CorrelatableFrame for Vec<u8>` | `src/correlation.rs:23`                           | Removed in the breaking release under roadmap `11.2.2`, reviewed under `14.2.1`     |
| `Packet for Vec<u8>`            | `src/connection/test_support.rs:35`               | Stays, test-only, behind `cfg(all(not(loom), any(test, feature = "test-support")))` |
| Client preamble leftovers       | `src/client/mod.rs:84`, `src/rewind_stream.rs:28` | Retained as `Vec<u8>` per ADR 009                                                   |

## Before and after

The examples are ordered as a reader would meet them: values first, then the
server-side editor, then the client-side editor, then the serializer, then the
codec. Examples using `PayloadBytes` are **Proposed**; examples using only
today's types are current behaviour and are shown so the final guide can
contrast them.

### Packets

Current:

```rust,ignore
use wireframe::app::PacketParts;

let parts = PacketParts::new(7, Some(correlation), payload_vec);
assert_eq!(parts.payload_bytes(), &payload_vec[..]);
```

**Proposed**:

```rust,ignore
use wireframe::app::PacketParts;

// Zero-copy: the payload is handed over without a vector round-trip.
let parts = PacketParts::new(7, Some(correlation), payload_bytes);
assert_eq!(parts.payload_bytes(), payload_bytes.as_ref());

// Compatibility only: an existing vector caller converts at the boundary.
let parts = PacketParts::new(7, Some(correlation), PayloadBytes::from_vec(payload_vec));
```

The compatibility line is the whole point of the helper set: it lets a caller
keep a `Vec<u8>` in hand while the library stops treating it as the primary
representation.

### Middleware

Current:

```rust,ignore
fn tag_request(request: &mut ServiceRequest) {
    request.frame_mut().push(b'!');
}
```

**Proposed**:

```rust,ignore
// Read-only middleware: no copy, no allocation.
fn peek_request(request: &ServiceRequest) -> &[u8] {
    request.frame()
}

// Mutating middleware: asks for an editor, which copies only on demand.
fn tag_request(request: &mut ServiceRequest) {
    let mut editor = request.edit();
    editor.push(b'!');
}
```

The `edit()` name is illustrative. Roadmap item `12.1.2` owns the final
ergonomics, and ADR 009 only settles that `frame_mut` is removed. The final
guide must publish whatever that item lands, not this sketch.

### Client hooks

Current:

```rust,ignore
client.before_send(|bytes: &mut Vec<u8>| {
    bytes.extend_from_slice(&header);
});
```

**Proposed**:

```rust,ignore
// New surface: an editor that copies only when the hook mutates.
client.before_send(|payload: &mut PayloadEditor| {
    payload.extend_from_slice(&header);
});

// Compatibility: an unchanged vector-taking closure keeps working.
client.before_send_from_vec_fn(|bytes: &mut Vec<u8>| {
    bytes.extend_from_slice(&header);
});
```

The adapter is accepted policy under ADR 009. The editor type is not; it
depends on roadmap item `3.2.1`.

### Serializers

Current:

```rust,ignore
let bytes: Vec<u8> = serializer.serialize(&message)?;
let frame = codec.wrap_payload(bytes::Bytes::from(bytes));
```

**Proposed** (accepted direction):

```rust,ignore
let bytes: PayloadBytes = serializer.serialize(&message)?;
let frame = codec.wrap_payload(bytes.into_bytes());
```

**Proposed** (amendment-dependent, not accepted):

```rust,ignore
// Only if the ADR 009 amendment is accepted: a deprecated shim for callers
// that cannot yet take the wrapper.
let bytes: Vec<u8> = serializer.serialize_to_vec(&message)?;
```

Publishing the second block as guidance before roadmap item `11.1.2.1`
completes would state an unaccepted API as policy. It stays labelled until then.

### Custom codecs

Current, for a frame that owns a vector:

```rust,ignore
struct MyFrame {
    payload: Vec<u8>,
}

impl FrameCodec for MyCodec {
    type Frame = MyFrame;

    fn frame_payload(frame: &MyFrame) -> &[u8] { &frame.payload }

    fn wrap_payload(&self, payload: Bytes) -> MyFrame {
        MyFrame { payload: payload.to_vec() }
    }
}
```

Proposed, for a frame that shares the buffer:

```rust,ignore
struct MyFrame {
    payload: Bytes,
}

impl FrameCodec for MyCodec {
    type Frame = MyFrame;

    fn frame_payload(frame: &MyFrame) -> &[u8] { frame.payload.as_ref() }

    // Without this override the default copies the payload on extraction.
    fn frame_payload_bytes(frame: &MyFrame) -> Bytes { frame.payload.clone() }

    fn wrap_payload(&self, payload: Bytes) -> MyFrame {
        MyFrame { payload }
    }
}
```

This is the only set of examples that can be published today without an
unimplemented type, because `Bytes` is already the trait's currency. ADR 009
asks that conversion stay at the edge of legacy callers rather than inside
codecs, which is why the example has no per-codec vector constructor.

## Compatibility helpers and their cost

ADR 009 fixes both the helper set and its lifetime. The cost column is what
most migration guides omit, and it is the part downstream users need.

| Helper                        | Direction      | Cost                                                      | Removal                                    |
| ----------------------------- | -------------- | --------------------------------------------------------- | ------------------------------------------ |
| `PayloadBytes::from_vec`      | `Vec<u8>` in   | May allocate a shared control block; may reuse the vector | Second breaking release after introduction |
| `PayloadBytes::into_vec`      | `Vec<u8>` out  | Allocates a fresh vector when the buffer is shared        | Same                                       |
| `serialize_to_vec`            | serializer out | One vector allocation per call                            | Same, if it ships at all                   |
| `BeforeSendHook::from_vec_fn` | hook adapter   | Copies the payload when the wrapped closure mutates       | Same                                       |

Removal timing follows ADR 009's rule: helpers introduced in breaking release
`0.N` are removed in `0.N+2`; after Wireframe reaches 1.0 the equivalent rule is
`1.x` to `2.x`. Every helper carries a `#[deprecated(since, note)]` attribute
whose note names both the migration path and the removal version, so a consumer
who suppresses the warning still learns the schedule from `cargo doc`.

The removal decision is reviewed, not automatic. Roadmap item `14.2.1`
re-evaluates each retained helper and either accepts it as a narrow permanent
shim or schedules its removal. The review is recorded, so a helper cannot
become permanent by inattention.

## Retained preamble-buffer policy

ADR 009 keeps client preamble leftovers on owned `Vec<u8>` for the breaking
release, and roadmap item `12.2.2` re-evaluates them afterwards. The reason is
structural rather than historical: the preamble exchange reads ahead of the
server's response, hands the surplus bytes back to the connection, and the
`RewindStream` replays them before any frame decoding happens
(`src/client/mod.rs:84`, `src/rewind_stream.rs:28`). Those bytes are not a
frame payload, so converting them to the payload representation would create a
conversion at both ends of a path that never reaches a codec.

The final guide must state the retained policy positively: preamble leftovers
stay `Vec<u8>` in this release, they are copied once at the rewind boundary,
and the copy is not on the zero-copy default path. It must also state that
roadmap item `12.2.2` will either confirm that decision or migrate the path
consistently with the rest of the client surface.

## Policy ambiguities to resolve before publishing guidance

These must be closed before roadmap item `13.1.2` publishes the guide. Each
names an owner item so the resolution is traceable.

| Ambiguity                                                          | Why it blocks guidance                                                                | Owner item |
| ------------------------------------------------------------------ | ------------------------------------------------------------------------------------- | ---------- |
| Serializer amendment accepted or rejected                          | Decides whether `serialize_to_vec` exists and which release removes it                | `11.1.2.1` |
| Whether `serialize_to_vec` ships even if the amendment is accepted | ADR 009 says "if it proves necessary"; that is a judgement, not a rule                | `11.1.2.1` |
| `Envelope` bincode derive versus wrapper traits                    | Decides whether `Envelope`'s wire encoding changes, which is a compatibility question | `3.1.1`    |
| Middleware editor name and shape                                   | ADR 009 removes `frame_mut` but does not name its replacement                         | `12.1.2`   |
| Whether `into_inner` survives on middleware wrappers               | Removing it is a second break beyond the editor change                                | `12.1.2`   |
| Target release version                                             | Decides every deprecation `since` and removal version string in the guide             | `14.1.1`   |

The version row is not pedantry. `Cargo.toml` currently declares `0.3.0`, and
the repository already carries a
[v0.3.0 to v0.4.0 migration guide](v0-3-0-to-v0-4-0-migration-guide.md) for the
prepared-application transition. Whether the zero-copy break joins that release
or follows it changes every `#[deprecated(since = "...")]` string in the helper
set. The [release plan](zero-copy-release-plan.md) records the determination
and its evidence.

## Downstream breakpoint checklist

The final guide is complete when, for each row below, it states the old
signature, the new signature, the compile error a caller will see, and the
shortest correct fix.

- [ ] `PacketParts::new` and `Envelope::new` payload argument
- [ ] `PacketParts::into_payload` return type
- [ ] `ServiceRequest::frame_mut`, `ServiceResponse::frame_mut`
- [ ] `ServiceRequest::into_inner`, `ServiceResponse::into_inner`
- [ ] `BeforeSendHook` closure argument
- [ ] `RequestHooks::before_send` bound
- [ ] `Serializer::serialize` return type
- [ ] `SerdeSerializerBridge::serialize_serde` return type
- [ ] `EncodeWith::encode_with` return type
- [ ] `CorrelatableFrame for Vec<u8>` removal
- [ ] `frame_payload_bytes` default-implementation copy

## References

- [ADR 008: zero-copy public byte container](adr-008-zero-copy-public-byte-container.md)
- [ADR 009: `Vec<u8>` migration rollout](adr-009-vec-u8-migration-rollout.md)
- [ADR 010: transport frame boundary](adr-010-transport-frame-boundary-for-zero-copy.md)
- [Frame and `Vec<u8>` inventory](frame-vec-u8-inventory.md)
- [Serializer output migration plan](execplans/issue-538-plan-zero-copy-serializer-output-migration.md)
- [Zero-copy release plan](zero-copy-release-plan.md)
- [v0.3.0 to v0.4.0 migration guide](v0-3-0-to-v0-4-0-migration-guide.md)
