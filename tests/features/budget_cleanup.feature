@budget_cleanup
Feature: Budget cleanup and reclamation semantics
  Memory budgets are reclaimed when partial assemblies time out or
  complete, allowing new frames to arrive within the freed headroom.
  When a connection closes, all partial assemblies are freed via RAII.

  Scenario: Timeout purge reclaims budget for subsequent frames
    Given a cleanup app configured as 200/2048/10/10
    When a cleanup first frame for key 1 with body "aaaaaaaa" arrives
    And cleanup virtual time advances by 201 milliseconds
    And a cleanup first frame for key 2 with body "bbbbbbbb" arrives
    And a cleanup final continuation for key 2 sequence 1 with body "cc" arrives
    Then cleanup payload "bbbbbbbbcc" is eventually received
    And no cleanup connection error is recorded

  Scenario: Completed assembly reclaims budget for subsequent frames
    Given a cleanup app configured as 200/2048/10/10
    When a cleanup first frame for key 3 with body "cccccccc" arrives
    And a cleanup final continuation for key 3 sequence 1 with body "d" arrives
    Then cleanup payload "ccccccccd" is eventually received
    When a cleanup first frame for key 4 with body "eeeeeeee" arrives
    And a cleanup final continuation for key 4 sequence 1 with body "f" arrives
    Then cleanup payload "eeeeeeeef" is eventually received
    And no cleanup connection error is recorded

  Scenario: Connection close frees all partial assemblies
    Given a cleanup app configured as 200/2048/100/100
    When a cleanup first frame for key 5 with body "aaa" arrives
    And a cleanup first frame for key 6 with body "bbb" arrives
    And the cleanup client disconnects
    Then no cleanup connection error is recorded
