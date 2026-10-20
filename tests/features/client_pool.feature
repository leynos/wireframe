Feature: Client pool
  The wireframe client pool reuses warm sockets, enforces admission limits,
  recycles idle sockets, and discards sockets after cancelled operations.

  Scenario: Client pool warm reuse preserves preamble state
    Given client pool warm reuse is configured
    When client pool warm reuse runs against one socket
    Then client pool warm reuse preserves one preamble handshake

  Scenario: Client pool enforces per-socket in-flight limits
    Given client pool in-flight limiting is configured
    When client pool acquires more leases than one socket allows
    Then client pool blocks the extra acquire until capacity returns

  Scenario: Client pool rejects acquires above the waiter limit
    Given client pool waiter admission is bounded
    When client pool holds every permit and exceeds its waiter limit
    Then client pool blocks an allowed waiter and rejects the excess acquire

  Scenario: Client pool recycles idle sockets
    Given client pool idle recycling is configured
    When client pool idles past the recycle timeout
    Then client pool reconnects and replays the preamble

  Scenario: Client pool reconnects after a cancelled call
    Given client pool cancellation safety is configured
    When client pool cancels a delayed in-flight call
    Then client pool reconnects after cancellation and releases capacity

  Scenario: Client pool rotates slots and waits when every slot is leased
    Given client pool slot rotation is configured with two slots
    When client pool acquires leases through the public API
    Then client pool holds two distinct physical sockets
    And client pool leases distinct slots in rotation order
    And client pool releases capacity when a lease is dropped
