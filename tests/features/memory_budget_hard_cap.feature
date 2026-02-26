@memory_budget_hard_cap
Feature: Hard-cap memory budget connection abort
  When buffered assembly bytes exceed the aggregate memory budget, frames
  are rejected and the connection is eventually terminated.

  Scenario: Connection terminates after repeated budget violations
    Given a hard-cap inbound app configured as 200/2048/8/8
    When hard-cap first frames for keys 1 to 11 each with body "aaaaaaaa" arrive
    Then the hard-cap connection terminates with an error

  Scenario: Connection survives when frames stay within budget
    Given a hard-cap inbound app configured as 200/2048/100/100
    When a hard-cap first frame for key 12 with body "cc" arrives
    And a hard-cap final continuation for key 12 sequence 1 with body "dd" arrives
    Then hard-cap payload "ccdd" is eventually received
    And no hard-cap connection error is recorded
