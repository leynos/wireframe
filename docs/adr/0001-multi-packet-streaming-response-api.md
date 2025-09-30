# ADR 0001: Multi-Packet Streaming Response API

## Status

Accepted

## Context

The roadmap for Wireframe 1.0 prioritises improving ergonomics around
multi-packet streaming responses so handlers can deliver zero, one, or many
frames per logical request without bespoke plumbing.[^roadmap-phase6] Existing
infrastructure already supports correlated frames, end-of-stream markers, and
connection-actor back-pressure, but the public API leaves developers to wire up
`Response::MultiPacket` manually. The current design document therefore needs a
codified decision describing how channel-backed streaming is exposed to
handlers and the roadmap requires actionable tasks to drive the implementation.

## Decision

Handlers MAY return a tuple consisting of a Tokio `mpsc::Sender` and an initial
`Response` to initiate a multi-packet stream. The framework will:

- create ergonomic helpers (for example `Response::with_channel`) that yield
  the sender alongside a `Response::MultiPacket` wrapping the paired receiver;
- emit any immediate frames contained in the returned `Response` before
  streaming additional frames pulled from the receiver; and
- rely on Tokio channel semantics for back-pressure and cancellation, ensuring
  the producer suspends when buffers are full and graceful termination occurs
  when all senders drop.

This decision keeps single-frame handlers unchanged whilst embracing the
existing connection actor and protocol hook lifecycle.

## Consequences

- Documentation describing multi-packet streaming MUST reflect the tuple-based
  API and explain helper constructors that prepare the channel pair.
- The development roadmap gains concrete tasks covering helper construction,
  handler ergonomics, and documentation updates so the work is tracked
  explicitly.
- Implementations MUST continue to send protocol-specific end-of-stream
  markers when the receiver closes and preserve correlation identifiers across
  all emitted frames.

[^roadmap-phase6]:
    See [roadmap Phase 6 in the development roadmap](../roadmap.md).
