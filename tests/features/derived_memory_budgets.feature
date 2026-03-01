@derived_memory_budgets
Feature: Derived memory budget defaults from buffer capacity
  When memory budgets are not set explicitly, sensible defaults are derived
  from the codec buffer capacity, enabling all three tiers of protection.

  Scenario: Derived budgets enforce per-connection limit
    Given a derived-budget app with buffer capacity 512
    When derived-budget first frames for keys 1 to 95 each with body size 400 arrive
    Then the derived-budget connection terminates with an error

  Scenario: Derived budgets allow frames within limits
    Given a derived-budget app with buffer capacity 512
    When a derived-budget first frame for key 1 with body "bb" arrives
    And a derived-budget final continuation for key 1 sequence 1 with body "cc" arrives
    Then derived-budget payload "bbcc" is eventually received
    And no derived-budget connection error is recorded

  Scenario: Explicit budgets override derived defaults
    Given a derived-budget app with buffer capacity 512 and explicit budgets 2048/8/8
    When derived-budget first frames for keys 1 to 11 each with body "aaaaaaaa" arrive
    Then the derived-budget connection terminates with an error
