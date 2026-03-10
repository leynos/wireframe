Feature: Client pool
  The wireframe client pool reuses warm sockets, enforces admission limits,
  and recycles idle sockets.

  Scenario: Client pool warm reuse preserves preamble state
    Given client pool warm reuse is configured
    When client pool warm reuse runs against one socket
    Then client pool warm reuse preserves one preamble handshake

  Scenario: Client pool enforces per-socket in-flight limits
    Given client pool in-flight limiting is configured
    When client pool acquires more leases than one socket allows
    Then client pool blocks the extra acquire until capacity returns

  Scenario: Client pool recycles idle sockets
    Given client pool idle recycling is configured
    When client pool idles past the recycle timeout
    Then client pool reconnects and replays the preamble
