@budget_cleanup
Feature: Budget cleanup and reclamation semantics
  Memory budgets are reclaimed when partial assemblies time out or
  complete, allowing new frames to arrive within the freed headroom.
  When a connection closes, all partial assemblies are freed via RAII.

  Scenario: Timeout purge reclaims budget for subsequent frames
    Given a cleanup app configured as 200/2048/12/12
    When a cleanup first frame for key 1 with body "aaaaaaaaa" arrives
    And the cleanup server processes the first frame
    And cleanup waits 201 milliseconds
    And a cleanup first frame for key 2 with body "bbbbbbbbb" arrives
    And a cleanup final continuation for key 2 sequence 1 with body "c" arrives
    Then cleanup payload "bbbbbbbbbc" is eventually received
    And no cleanup connection error is recorded

  Scenario: Completed assembly reclaims budget for subsequent frames
    Given a cleanup app configured as 200/2048/12/12
    When a cleanup first frame for key 3 with body "ccccccccc" arrives
    And a cleanup final continuation for key 3 sequence 1 with body "d" arrives
    Then cleanup payload "cccccccccd" is eventually received
    When a cleanup first frame for key 4 with body "eeeeeeeee" arrives
    And a cleanup final continuation for key 4 sequence 1 with body "f" arrives
    Then cleanup payload "eeeeeeeeef" is eventually received
    And no cleanup connection error is recorded

  Scenario: Connection close frees all partial assemblies
    Given a cleanup app configured as 200/2048/100/100
    When a cleanup first frame for key 5 with body "aaa" arrives
    And a cleanup first frame for key 6 with body "bbb" arrives
    And the cleanup client disconnects
    Then no cleanup connection error is recorded
