Feature: Codec test fixtures
  The wireframe_testing crate provides codec fixture functions for
  generating valid and invalid Hotline-framed wire bytes for testing.

  Scenario: Valid fixture decodes to expected payload
    Given a Hotline codec allowing fixtures up to 4096 bytes
    When a valid fixture frame is decoded
    Then the decoded payload matches the fixture input

  Scenario: Oversized fixture is rejected by decoder
    Given a Hotline codec allowing fixtures up to 4096 bytes
    When an oversized fixture frame is decoded
    Then the decoder reports an invalid data error

  Scenario: Truncated fixture produces a decode error
    Given a Hotline codec allowing fixtures up to 4096 bytes
    When a truncated fixture frame is decoded
    Then the decoder reports bytes remaining on stream

  Scenario: Correlated fixtures share the same transaction identifier
    Given a Hotline codec allowing fixtures up to 4096 bytes
    When correlated fixture frames are decoded
    Then all frames have the expected transaction identifier
