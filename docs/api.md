# API overview

## PacketParts

A `PacketParts` struct decomposes a packet into its components:

<!-- tested-example: api-001 -->
```rust
use wireframe::app::PacketParts;

let parts = PacketParts::new(7, Some(42), vec![1, 2, 3]);
```

- `id: u32` — envelope identifier used for routing (`packet` in trait
  abstraction contexts)
- `correlation_id: Option<u64>` — `None` marks an unsolicited event or
  server‑initiated push
- `payload: Vec<u8>` — raw message bytes

Custom packet types can convert to and from `PacketParts` to avoid manual
mapping:

<!-- tested-example: api-002 -->
```rust
use wireframe::app::{Envelope, PacketParts};

let parts = PacketParts::new(7, None, vec![1, 2, 3]);
let _env = Envelope::from(parts);
```

`None` propagation ensures packets that originate on the server carry no
accidental correlation identifier.
