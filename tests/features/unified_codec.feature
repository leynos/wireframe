Feature: Unified codec pipeline
  All outbound frames pass through the FramePipeline before reaching the wire,
  ensuring consistent fragmentation and metrics regardless of response origin.

  Scenario: Handler response round-trips through the unified pipeline
    Given a wireframe echo server with a buffer capacity of 512 bytes
    When the client sends a 5-byte payload
    Then the handler receives the original payload
    And the client receives a response matching the original payload

  Scenario: Fragmented response passes through the unified pipeline
    Given a wireframe echo server with a buffer capacity of 512 bytes and fragmentation enabled
    When the client sends a fragmented 1200-byte payload
    Then the handler receives the reassembled payload
    And the client receives a fragmented response matching the original payload

  Scenario: Small payload passes through the pipeline unfragmented
    Given a wireframe echo server with a buffer capacity of 512 bytes and fragmentation enabled
    When the client sends a 16-byte payload
    Then the handler receives the original payload
    And the client receives an unfragmented response matching the original payload

  Scenario: Multiple sequential requests pass through the pipeline
    Given a wireframe echo server with a buffer capacity of 512 bytes
    When the client sends 5 sequential 8-byte payloads
    Then the handler receives all 5 payloads in order
    And the client receives 5 responses matching the original payloads

  Scenario: Disabled fragmentation passes large payloads unchanged
    Given a wireframe echo server with a buffer capacity of 512 bytes
    When the client sends a 256-byte payload
    Then the handler receives the original payload
    And the client receives an unfragmented response matching the original payload
