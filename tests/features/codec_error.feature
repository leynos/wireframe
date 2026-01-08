Feature: Codec error taxonomy and recovery
  The codec layer provides structured error handling with recovery policies.

  Scenario: Clean EOF at frame boundary
    Given a wireframe server with default codec
    When a client connects and sends a complete frame
    And the client closes the connection cleanly
    Then the server detects a clean EOF

  Scenario: Premature EOF mid-frame
    Given a wireframe server with default codec
    When a client connects and sends partial frame data
    And the client closes the connection abruptly
    Then the server detects a mid-frame EOF with partial data

  Scenario: Oversized frame produces framing error
    Given a wireframe server with max frame length 64 bytes
    When a client sends a frame larger than 64 bytes
    Then the server rejects the frame with an oversized error

  Scenario: Recovery policy defaults
    Given a codec error of type framing with variant oversized
    Then the default recovery policy is drop
    Given a codec error of type eof with variant clean_close
    Then the default recovery policy is disconnect
