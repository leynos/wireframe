# MessageAssembler hook trait and per-frame header parsing

This ExecPlan is a living document. The sections `Progress`,
`Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must
be kept up to date as work proceeds.

No `PLANS.md` exists in this repository as of 2026-01-04.

## Purpose / Big Picture

Wireframe needs a protocol-facing hook for multi-frame request assembly and a
shared header model that distinguishes “first frame” from “continuation frame”.
This work delivers the `MessageAssembler` trait and the first version of header
parsing types so protocol crates can implement consistent parsing and Wireframe
can later apply shared buffering and assembly logic.

Success is observable when:

- The public `MessageAssembler` trait and frame header types are available in
  `wireframe::message_assembler` and re-exported in `wireframe`.
- A protocol can implement `MessageAssembler` to parse a simple header into a
  `FrameHeader::First` or `FrameHeader::Continuation` value.
- Unit tests cover first/continuation parsing, short-header errors, and header
  length accounting.
- A Cucumber feature validates the same behaviours via the behavioural test
  harness.
- `docs/users-guide.md` explains the new public interface and how to configure
  it (including any “not yet wired into the connection path” caveats).
- The relevant design docs capture the interface decisions.
- `docs/roadmap.md` marks 8.2.1 and 8.2.2 as done once the change lands.

## Progress

- [x] (2026-01-04 00:00Z) Draft ExecPlan for 8.2.1 and 8.2.2.
- [x] (2026-01-04 01:00Z) Define `MessageAssembler` trait and header types under
  `src/message_assembler/` with module-level docs and examples.
- [x] (2026-01-04 01:00Z) Add builder support to store an optional assembler in
  `src/app/builder.rs`.
- [x] (2026-01-04 01:05Z) Add unit tests for header parsing and error handling.
- [x] (2026-01-04 01:10Z) Add Cucumber feature, world, and steps for message
  assembler parsing.
- [x] (2026-01-04 01:15Z) Update design docs, user guide, and roadmap
  checkboxes.
- [x] (2026-01-04 02:05Z) Run formatting, linting, and test gates with `make`
  targets.

## Surprises & Discoveries

- Observation: Streaming request primitives exist (`src/request/mod.rs` and
  `src/extractor.rs`), but the inbound pipeline in `src/app/connection.rs` does
  not yet invoke a message-assembly hook. This plan keeps integration scoped to
  8.2.1/8.2.2 and documents the limitation in the user guide until 8.2.5.
  Evidence: `WireframeApp::handle_frame` decodes to `Envelope` and dispatches
  directly to handlers after fragmentation reassembly.

## Decision Log

- Decision: Introduce a dedicated public module
  `src/message_assembler/` for the hook trait and header types. Rationale:
  Keeps assembly concerns distinct from `hooks` and allows clear documentation
  of the protocol-facing interface. Date/Author: 2026-01-04 (Codex).
- Decision: Model frame headers with a public `FrameHeader` enum and
  `FirstFrameHeader`/`ContinuationFrameHeader` structs plus newtypes for
  `MessageKey` and optional `FrameSequence`. Rationale: Avoids integer soup and
  aligns with the ADR’s required header fields while leaving room for
  8.2.3/8.2.4. Date/Author: 2026-01-04 (Codex).
- Decision: Use `std::io::Error` for parse failures in the trait method.
  Rationale: Matches the existing framing/codec error model and keeps the trait
  object-safe without adding another type parameter to `WireframeApp`.
  Date/Author: 2026-01-04 (Codex).

## Outcomes & Retrospective

Not started yet.

## Context and Orientation

Wireframe’s inbound path decodes frames into `Envelope` values inside
`src/app/connection.rs`, then optionally applies transport fragmentation
reassembly via `src/app/fragmentation_state.rs` and
`frame_handling::reassemble_if_needed`. Handlers receive buffered payloads
through `ServiceRequest` and `PacketParts` in `src/middleware.rs`.

Streaming request types (`RequestParts`, `RequestBodyStream`, and the
`StreamingBody` extractor) live in `src/request/mod.rs` and `src/extractor.rs`,
but they are not yet wired into the connection loop. The `MessageAssembler`
hook introduced here is the protocol-facing interface for that forthcoming
integration (see Architecture Decision Record (ADR) 0002).

Key references:

- `docs/adr-002-streaming-requests-and-shared-message-assembly.md`
  (authoritative requirements for `MessageAssembler`).
- `docs/generic-message-fragmentation-and-re-assembly-design.md` section 9
  (composition order and memory budgeting).
- `docs/multi-packet-and-streaming-responses-design.md` section 11.4
  (MessageAssembler composition narrative).
- `docs/the-road-to-wireframe-1-0-feature-set-`
  `philosophy-and-capability-maturity.md` section “Protocol-level message
  assembly”.
- `docs/hardening-wireframe-a-guide-to-production-resilience.md` (resource
  limits and failure semantics).
- Testing guidance: `docs/rust-testing-with-rstest-fixtures.md`,
  `docs/behavioural-testing-in-rust-with-cucumber.md`,
  `docs/reliable-testing-in-rust-via-dependency-injection.md`, and
  `docs/rust-doctest-dry-guide.md`.

## Plan of Work

Start by adding a new `message_assembler` module that defines the
`MessageAssembler` trait, header types, and newtypes for message keys and
(optional) sequence indices. Document the trait with examples that show how to
parse a header using `bytes::Buf` rather than indexing. Next, wire the builder
so applications can register an assembler via
`WireframeApp::with_message_assembler` (and access it via
`message_assembler()`), even though the runtime integration lands in 8.2.5.
Then add unit tests for parsing behaviours and error handling using a
lightweight test assembler. Add a Cucumber feature that asserts the same
behaviours through the behavioural harness. Finally, update the design and user
documentation, and mark 8.2.1/8.2.2 as done in the roadmap.

## Concrete Steps

1. Read the core design documents listed above to confirm required header
   fields and the intended composition order.

2. Create `src/message_assembler/` with:

   - `src/message_assembler/mod.rs` containing the public API and module-level
     `//!` documentation.
   - `src/message_assembler/header.rs` defining `FrameHeader`,
     `FirstFrameHeader`, and `ContinuationFrameHeader` along with newtypes such
     as `MessageKey` and `FrameSequence`.
   - `src/message_assembler/error.rs` only if a shared error type is needed for
     examples or tests; otherwise, rely on `std::io::Error`.

   Keep each file below 400 lines and add doc examples for public items.

3. Add `pub mod message_assembler;` to `src/lib.rs`, and re-export the new
   trait and header types for a stable public surface.

4. Update `src/app/builder.rs`:

   - Add a `message_assembler: Option<Arc<dyn MessageAssembler>>` field.
   - Thread the field through `Default`, `rebuild_with_params`, and any
     builder transitions.
   - Add `with_message_assembler(...)` and `message_assembler()` methods.

5. Implement unit tests (suggested location:
   `src/message_assembler/tests.rs`):

   - Build a small `TestMessageAssembler` that parses a simple header format
     (one byte “kind” tag plus fixed-width fields for key and lengths).
   - Validate first-frame parsing returns `FrameHeader::First` with correct
     metadata length, body length, and header byte count.
   - Validate continuation parsing returns `FrameHeader::Continuation`.
   - Validate short/invalid headers return `io::ErrorKind::InvalidData`.
   - Avoid indexing and use `bytes::Buf` for parsing to satisfy clippy.

6. Add behavioural tests:

   - `tests/features/message_assembler.feature` with scenarios for first-frame
     and continuation parsing.
   - `tests/worlds/message_assembler.rs` storing the parsed header and errors.
   - `tests/steps/message_assembler_steps.rs` with Given/When/Then steps.
   - Register the world in `tests/worlds/mod.rs`, `tests/world.rs`, and run it
     from `tests/cucumber.rs`. Add the steps module in `tests/steps/mod.rs`.

7. Update documentation:

   - `docs/users-guide.md`: describe the new `MessageAssembler` trait, how to
     configure it via `WireframeApp`, and call out that runtime integration
     into the inbound pipeline arrives in 8.2.5.
   - `docs/adr-002-streaming-requests-and-shared-message-assembly.md`: record
     the concrete trait signature and header model decisions.
   - `docs/generic-message-fragmentation-and-re-assembly-design.md` and
     `docs/multi-packet-and-streaming-responses-design.md`: add references to
     the new API surface.
   - `docs/the-road-to-wireframe-1-0-feature-set-`
     `philosophy-and-capability-maturity.md`: note the concrete hook trait
     surface.

8. Update `docs/roadmap.md` to mark 8.2.1 and 8.2.2 as done when all above
   changes are complete.

## Validation and Acceptance

Acceptance requires all of the following:

- New public `MessageAssembler` trait and header types compile and are
  documented with examples.
- Unit tests cover parsing for first and continuation frames and error cases.
- Cucumber behavioural tests for message assembler parsing pass.
- Design docs, user guide, and roadmap reflect the new interface.

Run validation from the repository root (use `tee` to capture full output):

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

If Mermaid diagrams are edited or added, also run:

    set -o pipefail
    timeout 300 make nixie 2>&1 | tee /tmp/wireframe-nixie.log
    echo "nixie exit: $?"

## Idempotence and Recovery

All steps are additive and can be re-run safely. If a step fails, fix the
underlying issue and re-run only the affected command(s). Use the `tee` outputs
to locate the failure before retrying. Avoid destructive commands; if a local
change needs to be backed out, revert only the specific files edited for this
feature.

## Artifacts and Notes

Expected artefacts after completion:

- `src/message_assembler/` module with trait, header types, and tests.
- New Cucumber feature and world files under `tests/features/` and
  `tests/worlds/`.
- Updated design docs and user guide entries.

## Interfaces and Dependencies

At the end of this work, the following public interfaces must exist and be
re-exported from `src/lib.rs`:

- Module `wireframe::message_assembler` with a module-level `//!` comment.
- Newtypes:

    pub struct MessageKey(pub u64);
    pub struct FrameSequence(pub u32);

  Include `From<u64>`/`From<u32>` and `Copy`, `Clone`, `Debug`, `Eq`, and
  `Hash`.

- Header model:

        pub enum FrameHeader {
            First(FirstFrameHeader),
            Continuation(ContinuationFrameHeader),
        }

        pub struct FirstFrameHeader {
            pub message_key: MessageKey,
            pub metadata_len: usize,
            pub body_len: usize,
            pub total_body_len: Option<usize>,
            pub is_last: bool,
        }

        pub struct ContinuationFrameHeader {
            pub message_key: MessageKey,
            pub sequence: Option<FrameSequence>,
            pub body_len: usize,
            pub is_last: bool,
        }

  If header length accounting is needed for slicing, add a
  `ParsedFrameHeader { header_len, header }` wrapper.

- Hook trait:

        pub trait MessageAssembler: Send + Sync + 'static {
            fn parse_frame_header(
                &self,
                payload: &[u8],
            ) -> Result<ParsedFrameHeader, std::io::Error>;
        }

- Builder configuration:

    impl<S, C, E, F> WireframeApp<S, C, E, F> {
        pub fn with_message_assembler(
            self,
            assembler: impl MessageAssembler + 'static,
        ) -> Self;

        pub fn message_assembler(&self) -> Option<Arc<dyn MessageAssembler>>;
    }

If the builder uses `Arc<dyn MessageAssembler>`, add the required `use` and
thread it through `rebuild_with_params` so type transitions preserve the
configured assembler.

## Revision note

Add a revision note only when this ExecPlan is updated after implementation
work has started.
