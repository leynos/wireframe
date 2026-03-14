Feature: Codec error regressions through wireframe_testing
  The wireframe_testing crate should remain sufficient for asserting codec
  error taxonomy, recovery policies, and observability labels introduced by
  roadmap item 9.1.2.

  Scenario: Oversized payload is classified as framing drop and counted
    Given a codec error regression world
    When an oversized framing error is recorded with its default policy
    Then the observability handle reports 1 codec error for framing and drop

  Scenario: Clean EOF at frame boundary maps to eof disconnect
    Given a codec error regression world
    When the default codec observes a clean EOF at frame boundary
    Then the last EOF classification is clean close
    And the observability handle reports 1 codec error for eof and disconnect

  Scenario: Partial header and partial payload closures are distinguished
    Given a codec error regression world
    When the default codec observes EOF during a partial header
    And the default codec observes EOF during a partial payload
    Then the first partial EOF classification is mid header
    And the second partial EOF classification is mid frame

  Scenario: A custom recovery hook overrides the recorded policy
    Given a codec error regression world
    When a strict recovery hook records an oversized framing error
    Then the observability handle reports 1 codec error for framing and disconnect
    And the observability handle reports 0 codec error for framing and drop
