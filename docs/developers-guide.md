# Wireframe developers' guide

This guide defines the architectural vocabulary used across Wireframe source,
rustdoc, and user-facing documentation. Treat it as the naming contract for new
APIs and refactors.

## Layer model and glossary

| Layer                 | Canonical term    | Primary types                                    | Description                                                                         |
| --------------------- | ----------------- | ------------------------------------------------ | ----------------------------------------------------------------------------------- |
| Transport framing     | Frame             | `FrameCodec::Frame`, `LengthDelimitedFrameCodec` | Physical wire unit read from or written to the socket.                              |
| Routing envelope      | Packet / Envelope | `Packet`, `Envelope`, `PacketParts`              | Routable wrapper that carries route id, optional correlation id, and payload bytes. |
| Domain payload        | Message           | `message::Message`, extractor `Message<T>`       | Typed application data encoded into packet payload bytes.                           |
| Transport subdivision | Fragment          | `FragmentHeader`, `Fragmenter`, `Reassembler`    | Size-limited chunk used when a payload exceeds frame budget.                        |

## Naming invariants

- Frame terms belong to codec and socket boundaries only.
- Packet or envelope terms belong to routing and middleware request/response
  wrappers.
- Message terms belong to typed payload encode/decode concerns.
- Fragment terms belong to transport splitting and reassembly.
- Correlation identifiers are cross-layer metadata and may appear on frame,
  packet, and message-adjacent APIs when protocol metadata requires it.

## Allowed aliases and prohibited mixing

| Canonical term    | Allowed aliases              | Avoid in the same context                 |
| ----------------- | ---------------------------- | ----------------------------------------- |
| Frame             | wire frame, transport frame  | packet, envelope, message                 |
| Packet / Envelope | routable envelope            | frame (unless describing codec transport) |
| Message           | payload type, domain message | frame, fragment                           |
| Fragment          | fragment chunk               | packet, message                           |

## API and docs checklist

Use this checklist before merging API naming changes:

- Confirm the identifier name matches the owning layer.
- Ensure rustdoc examples use the same term as the symbol being documented.
- Verify `docs/users-guide.md` and migration notes describe the same meaning.
- Label cross-layer terms (for example, `correlation_id`) explicitly as shared
  metadata.

## Vocabulary normalization outcome (2026-02-20)

The 2026-02-20 normalization pass aligned docs and rustdoc terminology to this
model and did not rename public symbols. Existing API names (`FrameCodec`,
`Packet`, `Envelope`, `Message`, `Fragment*`) already map cleanly to separate
layers, so the implementation focused on clarifying boundaries rather than
introducing additional breaking changes.
