Feature: Test observability harness
  The wireframe_testing crate provides an observability handle that
  captures logs and metrics for deterministic test assertions.

  Scenario: Metrics are captured via the observability handle
    Given an observability harness is acquired
    When a codec error metric is recorded
    Then the codec error counter equals 1

  Scenario: Logs are captured via the observability handle
    Given an observability harness is acquired
    When a warning log is emitted
    Then the log buffer contains the expected message

  Scenario: Clear resets captured state
    Given an observability harness is acquired
    When a codec error metric is recorded
    And the observability state is cleared
    Then the codec error counter equals 0

  Scenario: Absent metrics return zero
    Given an observability harness is acquired
    Then the codec error counter equals 0
