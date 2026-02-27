Feature: Codec-aware test harness drivers
  The wireframe_testing crate provides codec-aware driver functions
  that handle frame encoding and decoding transparently for any
  FrameCodec implementation.

  Scenario: Payload round-trip through a custom codec driver
    Given a wireframe app configured with a Hotline codec allowing frames up to 4096 bytes
    When a test payload is driven through the codec-aware payload driver
    Then the response payloads are non-empty

  Scenario: Frame-level driver preserves codec metadata
    Given a wireframe app configured with a Hotline codec allowing frames up to 4096 bytes
    When a test payload is driven through the codec-aware frame driver
    Then the response frames contain valid transaction identifiers
