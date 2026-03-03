@budget_transitions
Feature: Budget pressure transitions and dimension interactions
  Memory budget protection tiers interact correctly under changing
  pressure levels: soft-limit pacing at 80 per cent, hard-cap abort
  at 100 per cent. The tightest aggregate dimension controls
  enforcement when per-connection and in-flight limits differ.

  Scenario: Soft pressure escalates to connection termination
    Given a transition app configured as 200/2048/10/10
    When transition first frames for keys 1 to 15 each with body "aa" arrive
    Then the transition connection terminates with an error

  Scenario: Recovery from soft limit after assembly completion
    Given a transition app configured as 200/2048/10/10
    When a transition first frame for key 20 with body "aaaaaaaa" arrives
    And a transition final continuation for key 20 sequence 1 with body "b" arrives
    Then transition payload "aaaaaaaab" is eventually received
    When a transition first frame for key 21 with body "cc" arrives
    And a transition final continuation for key 21 sequence 1 with body "d" arrives
    Then transition payload "ccd" is eventually received
    And no transition connection error is recorded

  Scenario: Tightest aggregate dimension controls enforcement
    Given a transition app configured as 200/2048/20/10
    When transition first frames for keys 30 to 44 each with body "aa" arrive
    Then the transition connection terminates with an error
