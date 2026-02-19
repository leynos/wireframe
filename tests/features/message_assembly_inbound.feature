Feature: Inbound message assembly integration
  Message assembly runs on the inbound connection path after transport
  fragmentation and before handler dispatch.

  Scenario: Interleaved assembly dispatches completed payloads
    Given an inbound app with message assembly timeout 200 milliseconds
    When a first frame for key 1 with body "A1" arrives
    And a first frame for key 2 with body "B1" arrives
    And a final continuation frame for key 1 sequence 1 with body "A2" arrives
    And a final continuation frame for key 2 sequence 1 with body "B2" arrives
    Then the handler eventually receives payload "A1A2"
    And the handler eventually receives payload "B1B2"
    And no send error is recorded

  Scenario: Ordering violations are rejected and recovery can complete
    Given an inbound app with message assembly timeout 200 milliseconds
    When a first frame for key 9 with body "ab" arrives
    And a continuation frame for key 9 sequence 1 with body "cd" arrives
    And a continuation frame for key 9 sequence 3 with body "ef" arrives
    And a final continuation frame for key 9 sequence 2 with body "gh" arrives
    Then the handler eventually receives payload "abcdgh"
    And the handler receives 1 payload
    And no send error is recorded

  Scenario: Timeout purges partial assembly before continuation arrives
    Given an inbound app with message assembly timeout 10 milliseconds
    When a first frame for key 7 with body "ab" arrives
    And time advances by 30 milliseconds
    And a final continuation frame for key 7 sequence 1 with body "cd" arrives
    Then the handler receives 0 payloads
    And no send error is recorded
