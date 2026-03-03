@partial-frame-feeding
Feature: Partial frame and fragment feeding utilities

  Scenario: Single payload survives byte-at-a-time chunked delivery
    Given a wireframe app with a Hotline codec allowing 4096-byte frames for partial feeding
    When a test payload is fed in 1-byte chunks
    Then the partial feeding response payloads are non-empty

  Scenario: Multiple payloads survive misaligned chunked delivery
    Given a wireframe app with a Hotline codec allowing 4096-byte frames for partial feeding
    When 2 test payloads are fed in 7-byte chunks
    Then the partial feeding response contains 2 payloads

  Scenario: Fragmented payload is delivered as fragment frames
    Given a wireframe app with a Hotline codec allowing 4096-byte frames for partial feeding
    And a fragmenter capped at 20 bytes per fragment for partial feeding
    When a 100-byte payload is fragmented and fed through the app
    Then the fragment feeding completes without error

  Scenario: Fragmented payload survives chunked delivery
    Given a wireframe app with a Hotline codec allowing 4096-byte frames for partial feeding
    And a fragmenter capped at 20 bytes per fragment for partial feeding
    When a 100-byte payload is fragmented and fed in 3-byte chunks
    Then the fragment feeding completes without error
