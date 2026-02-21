# API overview

## PacketParts

A `PacketParts` struct decomposes a packet into its components:

```rust
let parts = PacketParts::new(id, correlation_id, payload);
```

- `id: u32` — envelope identifier used for routing (`packet` in trait
  abstraction contexts)
- `correlation_id: Option<u64>` — `None` marks an unsolicited event or
  server‑initiated push
- `payload: Vec<u8>` — raw message bytes

Custom packet types can convert to and from `PacketParts` to avoid manual
mapping:

```rust
let parts = PacketParts::new(id, None, data);
let env = Envelope::from(parts);
```

`None` propagation ensures packets that originate on the server carry no
accidental correlation identifier.
