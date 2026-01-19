# Enable zero-copy payload extraction for codecs

This execution plan (ExecPlan) is a living document. The sections `Progress`,
`Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must
be kept up to date as work proceeds.

## Purpose / Big Picture

The codec layer currently forces payload extraction through `&[u8]` slices,
which works well for the default `LengthDelimitedFrameCodec` (whose `Frame`
type is already `Bytes`), but the example codecs (`HotlineFrameCodec`,
`MysqlFrameCodec`) store payloads in `Vec<u8>` and copy data on both decode and
wrap paths. This change introduces a `frame_payload_bytes` method to the
`FrameCodec` trait that returns `Bytes` directly, updates the example codecs to
use `Bytes` internally for zero-copy operation, and adds regression tests that
verify pointer equality to prove the receive buffer is reused. Success is
visible when payloads flow through decode and wrap paths without allocation or
copying, validated by pointer-equality assertions in unit tests.

## Progress

- [x] Add `frame_payload_bytes` method to `FrameCodec` trait in `src/codec.rs`.
- [x] Override `frame_payload_bytes` for `LengthDelimitedFrameCodec`.
- [x] Update `HotlineFrame` to use `Bytes` instead of `Vec<u8>`.
- [x] Update `HotlineAdapter` decoder to use `BytesMut::freeze()`.
- [x] Update `HotlineAdapter` encoder to work with `Bytes`.
- [x] Update `HotlineFrameCodec` to override `frame_payload_bytes`.
- [x] Update `MysqlFrame` to use `Bytes` instead of `Vec<u8>`.
- [x] Update `MysqlAdapter` decoder to use `BytesMut::freeze()`.
- [x] Update `MysqlAdapter` encoder to work with `Bytes`.
- [x] Update `MysqlFrameCodec` to override `frame_payload_bytes`.
- [x] Add zero-copy regression test for `LengthDelimitedFrameCodec`.
- [x] Add zero-copy regression test for `HotlineFrameCodec` decode.
- [x] Add zero-copy regression test for `HotlineFrameCodec` wrap.
- [x] Add zero-copy regression test for `MysqlFrameCodec` decode.
- [x] Add zero-copy regression test for `MysqlFrameCodec` wrap.
- [x] Update `docs/adr-004-pluggable-protocol-codecs.md` (Architecture Decision
      Record) with design decision.
- [x] Update `docs/users-guide.md` with zero-copy guidance.
- [x] Mark roadmap entry 9.1.3 as done.
- [x] Run formatting, lint, and test gates.

## Surprises & Discoveries

- The `HotlineAdapter` and `MysqlAdapter` encoders required no changes because
  `extend_from_slice` works with `Bytes` via `Deref<Target = [u8]>`.
- Integration tests in `tests/example_codecs.rs` needed updates to construct
  frames with `Bytes` payloads instead of `Vec<u8>`.
- The existing test `length_delimited_wrap_payload_reuses_bytes` already
  validated zero-copy for wrapping; the new tests extend coverage to
  `frame_payload_bytes` extraction and example codecs.

## Decision Log

- Decision: Add `frame_payload_bytes` as a new method rather than changing the
  return type of `frame_payload`. Rationale: Maintains backward compatibility
  with existing custom codec implementations that rely on `&[u8]`. The new
  method has a default implementation that copies, so existing codecs continue
  to work without modification. Date/Author: 2026-01-11 / Terry.

- Decision: Use `Bytes` instead of `Vec<u8>` for payload storage in example
  frame types. Rationale: `Bytes` is reference-counted and `clone()` is cheap
  (atomic increment), enabling zero-copy extraction. The trade-off is that
  example codecs become slightly more complex, but they serve as templates for
  production codecs where zero-copy matters. Date/Author: 2026-01-11 / Terry.

- Decision: Verify zero-copy via pointer equality in tests. Rationale: Pointer
  comparison is the only reliable way to prove that no copy occurred. Comparing
  content only proves correctness, not efficiency. Date/Author: 2026-01-11 /
  Terry.

## Outcomes & Retrospective

**Completed 2026-01-19.**

- The `frame_payload_bytes` method was added to `FrameCodec` with a default
  implementation that copies from `frame_payload()`, ensuring backward
  compatibility.
- `LengthDelimitedFrameCodec`, `HotlineFrameCodec`, and `MysqlFrameCodec` all
  override `frame_payload_bytes` to return `frame.payload.clone()` (cheap
  atomic increment).
- `HotlineFrame` and `MysqlFrame` now use `Bytes` payloads, and their decoders
  use `BytesMut::freeze()` for zero-copy extraction from the receive buffer.
- Seven zero-copy regression tests verify pointer equality across decode,
  wrap, and extraction paths.
- Documentation updated in ADR-004 (resolved decision) and users-guide.md
  (zero-copy guidance section).
- Roadmap milestone 9.1.3 marked complete.
- All formatting, lint, and test gates pass.

## Context and Orientation

### Dependencies

> **Prerequisite: ExecPlan 9.1.2 must be complete before starting this work.**
>
> This plan depends on the error taxonomy introduced in ExecPlan 9.1.2
> ("Introduce a CodecError taxonomy"). Specifically, the following items from
> 9.1.2 must be merged before beginning 9.1.3:
>
> - **Error propagation surfaces**: The `Codec(CodecError)` variant added to
>   `WireframeError` in `src/response.rs` and `src/app/error.rs`, which provides
>   the structured error pathway for codec failures.
> - **WireframeError extensions**: The `From<CodecError>` implementations and
>   updated `Display`/`Error` traits that allow codec errors to flow through the
>   existing error handling infrastructure.
>
> These are required so that any errors arising from the new
> `frame_payload_bytes`
> method can be surfaced through the established error taxonomy rather than
> falling back to generic `io::Error` handling.

### Current State

The `FrameCodec` trait in `src/codec.rs` defines:

```rust
pub trait FrameCodec: Send + Sync + Clone + 'static {
    type Frame: Send + Sync + 'static;
    type Decoder: Decoder<Item = Self::Frame, Error = io::Error> + Send;
    type Encoder: Encoder<Self::Frame, Error = io::Error> + Send;

    fn decoder(&self) -> Self::Decoder;
    fn encoder(&self) -> Self::Encoder;
    fn frame_payload(frame: &Self::Frame) -> &[u8];  // Returns borrowed slice
    fn wrap_payload(&self, payload: Bytes) -> Self::Frame;
    fn correlation_id(_frame: &Self::Frame) -> Option<u64> { None }
    fn max_frame_length(&self) -> usize;
}
```

### Default Codec (Already Zero-Copy)

`LengthDelimitedFrameCodec` in `src/codec.rs` uses `Bytes` as its frame type:

- `frame_payload(frame: &Bytes) -> &[u8]` returns `frame.as_ref()` (zero-copy
  borrow)
- `wrap_payload(&self, payload: Bytes) -> Bytes` returns `payload` directly
  (zero-copy)
- `LengthDelimitedDecoder::decode()` uses `BytesMut::freeze()` (zero-copy
  conversion)

### Example Codecs (Copy on Decode and Wrap)

`HotlineFrame` and `MysqlFrame` in `src/codec/examples.rs` use `Vec<u8>`:

```rust
pub struct HotlineFrame {
    pub transaction_id: u32,
    pub payload: Vec<u8>,  // Forces allocation
}
```

The `HotlineAdapter::decode()` method copies:
`let payload = frame_bytes.to_vec();`

The `HotlineFrameCodec::wrap_payload()` method copies:
`payload: payload.to_vec()`

### Primary Usage Site

The connection handling code in `src/app/connection.rs` calls
`F::frame_payload(frame)` to extract payload bytes for envelope parsing. This
works with `&[u8]` and would also work with `Bytes` via `as_ref()`.

### Existing Zero-Copy Test

The `length_delimited_wrap_payload_reuses_bytes` test in `src/codec/tests.rs`
already verifies `wrap_payload` reuses memory:

```rust
fn length_delimited_wrap_payload_reuses_bytes() {
    let codec = LengthDelimitedFrameCodec::new(128);
    let payload = Bytes::from(vec![9_u8; 4]);
    let frame = codec.wrap_payload(payload.clone());

    assert_eq!(payload.len(), frame.len());
    assert_eq!(payload.as_ref().as_ptr(), frame.as_ref().as_ptr());
}
```

### Key Files

Codec implementation:

- `src/codec.rs` - `FrameCodec` trait and `LengthDelimitedFrameCodec`
- `src/codec/examples.rs` - `HotlineFrameCodec` and `MysqlFrameCodec`
- `src/codec/tests.rs` - Codec unit tests

Documentation to update:

- `docs/roadmap.md` - Mark 9.1.3 done
- `docs/users-guide.md` - Custom codec guidance
- `docs/adr-004-pluggable-protocol-codecs.md` - Design decisions

## Plan of Work

### Phase 1: Add `frame_payload_bytes` Method to Trait

Add a new method to `FrameCodec` that returns `Bytes` for zero-copy extraction.
The default implementation copies from `frame_payload()` for backward
compatibility. Codecs that store `Bytes` internally override this method to
return `frame.payload.clone()` (cheap atomic increment).

### Phase 2: Update `LengthDelimitedFrameCodec`

Override `frame_payload_bytes` to return `frame.clone()`. Since the frame type
is already `Bytes`, this is a cheap clone (reference count increment).

### Phase 3: Update Example Codecs

1. Change `HotlineFrame::payload` from `Vec<u8>` to `Bytes`
2. Update `HotlineAdapter::decode()` to use `frame_bytes.freeze()` instead of
   `.to_vec()`
3. Update `HotlineAdapter::encode()` to work with `Bytes` (use
   `extend_from_slice`)
4. Update `HotlineFrameCodec::frame_payload()` to return `&frame.payload`
5. Override `HotlineFrameCodec::frame_payload_bytes()` to return
   `frame.payload.clone()`
6. Update `HotlineFrameCodec::wrap_payload()` to store `Bytes` directly
7. Apply the same changes to `MysqlFrame` and `MysqlFrameCodec`

### Phase 4: Add Zero-Copy Regression Tests

Add tests that verify pointer equality to prove zero-copy behaviour:

1. `frame_payload_bytes_reuses_memory_for_length_delimited` - Verify the
   extracted `Bytes` points to the same memory as the decoded frame
2. `hotline_decode_produces_zero_copy_payload` - Verify the decoded payload
   points into the receive buffer
3. `hotline_wrap_payload_reuses_bytes` - Verify `wrap_payload` stores the
   `Bytes` without copying
4. Equivalent tests for `MysqlFrameCodec`

### Phase 5: Documentation Updates

1. Add "Zero-copy payload extraction" section to Architecture Decision Record
   (ADR) 004 under "Resolved Decisions"
2. Update the "Custom frame codecs" section in `docs/users-guide.md` with
   guidance on implementing zero-copy codecs
3. Mark all 9.1.3 sub-items as done in `docs/roadmap.md`

## Concrete Steps

1. Update `src/codec.rs` to add the `frame_payload_bytes` method:

   Add after the `frame_payload` method in the `FrameCodec` trait:

   ```rust
   /// Extract the message payload bytes from a frame as owned [`Bytes`].
   ///
   /// This method enables zero-copy payload extraction for codecs whose frame
   /// type uses `Bytes` internally. The default implementation copies the
   /// slice returned by [`frame_payload`] into a new `Bytes` buffer.
   ///
   /// Override this method when the frame type can provide the payload
   /// without allocation.
   fn frame_payload_bytes(frame: &Self::Frame) -> Bytes {
       Bytes::copy_from_slice(Self::frame_payload(frame))
   }
   ```

2. Update `LengthDelimitedFrameCodec` implementation to override the method:

   Add to the `impl FrameCodec for LengthDelimitedFrameCodec` block:

   ```rust
   fn frame_payload_bytes(frame: &Self::Frame) -> Bytes { frame.clone() }
   ```

3. Update `src/codec/examples.rs` `HotlineFrame`:

   ```rust
   pub struct HotlineFrame {
       pub transaction_id: u32,
       pub payload: Bytes,  // Changed from Vec<u8>
   }
   ```

4. Update `HotlineAdapter::decode()`:

   Change the payload extraction from:

   ```rust
   let payload = frame_bytes.to_vec();
   ```

   To:

   ```rust
   let payload = frame_bytes.freeze();
   ```

5. Update `HotlineAdapter::encode()`:

   Change the payload write from:

   ```rust
   dst.extend_from_slice(&item.payload);
   ```

   To (no change needed - `extend_from_slice` works with `Bytes` via `Deref`):

   ```rust
   dst.extend_from_slice(&item.payload);
   ```

6. Update `HotlineFrameCodec` implementation:

   ```rust
   fn frame_payload(frame: &Self::Frame) -> &[u8] { &frame.payload }

   fn frame_payload_bytes(frame: &Self::Frame) -> Bytes { frame.payload.clone() }

   fn wrap_payload(&self, payload: Bytes) -> Self::Frame {
       HotlineFrame {
           transaction_id: 0,
           payload,  // No .to_vec()
       }
   }
   ```

7. Apply the same changes to `MysqlFrame` and `MysqlFrameCodec`.

8. Add zero-copy regression tests to `src/codec/tests.rs`:

   ```rust
   #[test]
   fn frame_payload_bytes_reuses_memory_for_length_delimited() {
       let codec = LengthDelimitedFrameCodec::new(128);
       let payload = Bytes::from(vec![1_u8, 2, 3, 4]);
       let frame = codec.wrap_payload(payload.clone());

       let extracted = LengthDelimitedFrameCodec::frame_payload_bytes(&frame);

       assert_eq!(
           frame.as_ref().as_ptr(),
           extracted.as_ref().as_ptr(),
           "frame_payload_bytes should return the same memory region"
       );
   }
   ```

9. Add example codec zero-copy tests (in `src/codec/examples.rs` test module or
   `tests/example_codecs.rs`):

   ```rust
   #[test]
   fn hotline_wrap_payload_reuses_bytes() {
       let codec = HotlineFrameCodec::new(128);
       let payload = Bytes::from(vec![5_u8; 8]);
       let frame = codec.wrap_payload(payload.clone());

       assert_eq!(
           payload.as_ref().as_ptr(),
           frame.payload.as_ref().as_ptr(),
           "wrap_payload should reuse the Bytes without copying"
       );
   }
   ```

10. Update `docs/adr-004-pluggable-protocol-codecs.md`:

    Add under "Resolved Decisions":

    ```markdown
    ### Zero-copy payload extraction (resolved 2026-01-19)

    A `frame_payload_bytes` method was added to `FrameCodec` to enable zero-copy
    payload extraction:

    - **New method**: `fn frame_payload_bytes(frame: &Self::Frame) -> Bytes`
    - **Default behaviour**: Copies from `frame_payload()` for backward
      compatibility
    - **Optimised implementations**: Return `frame.payload.clone()` for
      `Bytes`-backed frames

    Guidelines for custom codecs:

    1. Use `Bytes` instead of `Vec<u8>` for payload storage in frame types
    2. Use `BytesMut::freeze()` in decoders instead of `.to_vec()`
    3. Override `frame_payload_bytes` to return `frame.payload.clone()`
    4. In `wrap_payload`, store the `Bytes` directly without conversion

    Verification via pointer equality:

    ```rust
    let extracted = MyCodec::frame_payload_bytes(&frame);
    assert_eq!(frame.payload.as_ptr(), extracted.as_ptr());
    ```

11. Update `docs/users-guide.md`:

    Add to the "Custom frame codecs" section:

    ```markdown
    #### Zero-copy payload extraction

    For performance-critical codecs, use `Bytes` instead of `Vec<u8>` for
    payload storage and override `frame_payload_bytes`:

    ```rust
    pub struct MyFrame {
        pub metadata: u32,
        pub payload: Bytes,  // Use Bytes, not Vec<u8>
    }

    impl FrameCodec for MyCodec {
        // …

        fn frame_payload(frame: &Self::Frame) -> &[u8] { &frame.payload }

        fn frame_payload_bytes(frame: &Self::Frame) -> Bytes {
            frame.payload.clone()  // Cheap: atomic reference count increment
        }

        fn wrap_payload(&self, payload: Bytes) -> Self::Frame {
            MyFrame {
                metadata: 0,
                payload,  // Store directly, no copy
            }
        }
    }
    ```

    In the decoder, use `BytesMut::freeze()` instead of `.to_vec()`:

    ```rust
    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        // … parse header …
        let payload = src.split_to(payload_len).freeze();  // Zero-copy
        Ok(Some(MyFrame { metadata, payload }))
    }
    ```

12. Update `docs/roadmap.md`:

    Change:

    ```markdown
    - [ ] 9.1.3. Enable zero-copy payload extraction for codecs.
      - [ ] Update `FrameCodec::frame_payload` to return a `Bytes`-backed view
      - [ ] Update the default codec adapter to avoid `Bytes` to `Vec<u8>` copying
      - [ ] Add a regression test or benchmark to confirm payloads reuse the
            receive buffer where possible.
    ```

    To:

    ```markdown
    - [x] 9.1.3. Enable zero-copy payload extraction for codecs.
      - [x] Update `FrameCodec::frame_payload` to return a `Bytes`-backed view
      - [x] Update the default codec adapter to avoid `Bytes` to `Vec<u8>` copying
      - [x] Add a regression test or benchmark to confirm payloads reuse the
            receive buffer where possible.
    ```

13. Run validation gates:

    ```sh
    set -o pipefail
    timeout 300 make fmt 2>&1 | tee /tmp/wireframe-fmt.log
    echo "fmt exit: $?"

    set -o pipefail
    timeout 300 make markdownlint 2>&1 | tee /tmp/wireframe-markdownlint.log
    echo "markdownlint exit: $?"

    set -o pipefail
    timeout 300 make check-fmt 2>&1 | tee /tmp/wireframe-check-fmt.log
    echo "check-fmt exit: $?"

    set -o pipefail
    timeout 300 make lint 2>&1 | tee /tmp/wireframe-lint.log
    echo "lint exit: $?"

    set -o pipefail
    timeout 300 make test 2>&1 | tee /tmp/wireframe-test.log
    echo "test exit: $?"
    ```

## Validation and Acceptance

Acceptance is based on observable behaviour:

- `frame_payload_bytes` method exists on `FrameCodec` trait
- `LengthDelimitedFrameCodec::frame_payload_bytes` returns `frame.clone()`
- `HotlineFrame::payload` and `MysqlFrame::payload` use `Bytes` type
- `HotlineAdapter` and `MysqlAdapter` decoders use `BytesMut::freeze()`
- `HotlineFrameCodec` and `MysqlFrameCodec` override `frame_payload_bytes`
- Pointer equality tests pass for all codec types
- `make check-fmt`, `make lint`, and `make test` succeed
- ADR 004 documents the zero-copy design decision
- Users guide documents zero-copy implementation guidance
- Roadmap 9.1.3 items are marked done

## Idempotence and Recovery

All changes are safe to reapply. If a refactor breaks compilation, revert the
individual file changes and reapply the steps one by one. The new
`frame_payload_bytes` method has a default implementation, so existing custom
codecs continue to work without modification.

## Artifacts and Notes

Record key evidence here once available:

- `make fmt` log: `/tmp/wireframe-fmt.log`
- `make markdownlint` log: `/tmp/wireframe-markdownlint.log`
- `make check-fmt` log: `/tmp/wireframe-check-fmt.log`
- `make lint` log: `/tmp/wireframe-lint.log`
- `make test` log: `/tmp/wireframe-test.log`

## Interfaces and Dependencies

### New `frame_payload_bytes` Method

```rust
// src/codec.rs

pub trait FrameCodec: Send + Sync + Clone + 'static {
    // ... existing methods ...

    /// Extract the message payload bytes from a frame as owned [`Bytes`].
    fn frame_payload_bytes(frame: &Self::Frame) -> Bytes {
        Bytes::copy_from_slice(Self::frame_payload(frame))
    }
}
```

### Updated Example Frame Types

```rust
// src/codec/examples.rs

pub struct HotlineFrame {
    pub transaction_id: u32,
    pub payload: Bytes,  // Changed from Vec<u8>
}

pub struct MysqlFrame {
    pub sequence_id: u8,
    pub payload: Bytes,  // Changed from Vec<u8>
}
```

## Files to Modify

| File                                        | Change                                    |
| ------------------------------------------- | ----------------------------------------- |
| `src/codec.rs`                              | Add `frame_payload_bytes` method to trait |
| `src/codec/examples.rs`                     | Update frame types to use `Bytes`         |
| `src/codec/tests.rs`                        | Add zero-copy regression tests            |
| `tests/example_codecs.rs`                   | Update tests for `Bytes` payloads         |
| `docs/adr-004-pluggable-protocol-codecs.md` | Document zero-copy design                 |
| `docs/users-guide.md`                       | Add zero-copy implementation guidance     |
| `docs/roadmap.md`                           | Mark 9.1.3 as done                        |

## Revision note (required when editing an ExecPlan)

- 2026-01-19: Fixed markdown formatting issues: removed orphan closing fences
  in Concrete Steps items 10 and 11 that violated MD040 (fenced code blocks
  must have a language specified); updated placeholder date "2026-01-XX" to
  "2026-01-19" in ADR update snippet; expanded "ADR" acronym in Progress
  checklist item for `adr-004-pluggable-protocol-codecs.md`.
- 2026-01-19: Added explicit dependency note in "Context and Orientation"
  section stating that ExecPlan 9.1.2 must complete its error propagation
  surfaces and WireframeError extensions before 9.1.3 can begin.
- 2026-01-19: Removed "Draft ExecPlan" meta-item from Progress. Replaced line
  number references with type/function names for stability. Expanded "ADR"
  acronym on first use in Phase 5.
- 2026-01-11: Initial draft of ExecPlan for roadmap item 9.1.3.
