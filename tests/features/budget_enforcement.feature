@budget_enforcement
Feature: Memory budget enforcement during message assembly
  When memory budgets are configured on a Wireframe application, the message
  assembly subsystem actively rejects frames that would exceed any of the
  three budget dimensions — per-message, per-connection, and in-flight.

  Background:
    Given a budgeted assembly state configured as 1024/30/50/40

  Scenario: Accept frames within all budget limits
    When a first frame for key 1 with 10 body bytes is accepted
    Then the frame is accepted without error
    And total buffered bytes is 10

  Scenario: Reject first frame exceeding connection budget
    When a first frame for key 1 with 51 body bytes is rejected
    Then the error is "ConnectionBudgetExceeded"
    And the active assembly count is 0

  Scenario: Reject continuation exceeding in-flight budget
    When a first frame for key 1 with 30 body bytes is accepted
    Then the frame is accepted without error
    When a continuation for key 1 with sequence 1 and 11 body bytes is rejected
    Then the error is "InFlightBudgetExceeded"
    And the active assembly count is 0

  Scenario: Reclaim budget headroom after assembly completes
    When a first frame for key 1 with 35 body bytes is accepted
    Then total buffered bytes is 35
    When a final continuation for key 1 with sequence 1 and 5 body bytes completes the message
    Then total buffered bytes is 0
    When a first frame for key 2 with 35 body bytes is accepted
    Then the frame is accepted without error

  Scenario: Reclaim budget headroom after timeout purge
    Given the clock is at time zero
    When a first frame for key 1 with 35 body bytes is accepted at time zero
    Then total buffered bytes is 35
    When expired assemblies are purged at 31 seconds
    Then total buffered bytes is 0
    When a first frame for key 2 with 35 body bytes is accepted
    Then the frame is accepted without error

  Scenario: Single-frame message bypasses aggregate budgets
    When a first frame for key 1 with 35 body bytes is accepted
    Then total buffered bytes is 35
    When a single-frame message for key 2 with 100 body bytes is accepted
    Then the single-frame message completes immediately
    And total buffered bytes is 35
