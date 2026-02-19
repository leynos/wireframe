Feature: Property-based codec round-trip hardening
  Scenario: Generated default codec payload sequences round-trip
    Given generated codec checks run with 96 cases
    When generated boundary payload sequences are exercised for the default codec
    Then the default codec generated sequence checks succeed

  Scenario: Generated malformed default codec frames are rejected
    Given generated codec checks run with 128 cases
    When generated malformed frame checks are exercised for the default codec
    Then the default codec generated malformed checks succeed

  Scenario: Generated mock codec sequences keep state deterministic
    Given generated codec checks run with 96 cases
    When generated stateful sequence checks are exercised for the mock codec
    Then the mock codec generated sequence checks succeed
