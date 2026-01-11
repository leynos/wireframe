Feature: Message assembly multiplexing and continuity validation
  Wireframe supports multiplexed message assembly with continuity validation
  for interleaved frame streams from multiple logical messages.

  Background:
    Given a message assembly state with max size 1024 and timeout 30 seconds

  Scenario: Single message assembly completes successfully
    Given a first frame for key 1 with metadata "AB" and body "hello"
    When the first frame is accepted
    Then the assembly result is incomplete
    And the buffered count is 1
    When a final continuation for key 1 with sequence 1 and body " world" arrives
    Then the assembly completes with body "hello world"
    And the buffered count is 0

  Scenario: Single-frame message completes immediately
    Given a final first frame for key 1 with body "complete"
    When the first frame is accepted
    Then the assembly completes with body "complete"
    And the buffered count is 0

  Scenario: Interleaved messages assemble independently
    Given a first frame for key 1 with body "A1"
    And a first frame for key 2 with body "B1"
    When all first frames are accepted
    Then the buffered count is 2
    When a final continuation for key 1 with sequence 1 and body "A2" arrives
    Then key 1 completes with body "A1A2"
    And the buffered count is 1
    When a final continuation for key 2 with sequence 1 and body "B2" arrives
    Then key 2 completes with body "B1B2"
    And the buffered count is 0

  Scenario: Out-of-order continuation is rejected
    Given a first frame for key 1 with body "start"
    When the first frame is accepted
    And a continuation for key 1 with sequence 1 arrives
    Then the assembly result is incomplete
    When a continuation for key 1 with sequence 3 arrives
    Then the error is sequence mismatch expecting 2 but found 3
    And the buffered count is 0

  Scenario: Duplicate continuation is rejected
    Given a first frame for key 1 with body "start"
    When the first frame is accepted
    And a continuation for key 1 with sequence 1 arrives
    And a continuation for key 1 with sequence 2 arrives
    When a continuation for key 1 with sequence 1 arrives
    Then the error is duplicate frame for key 1 sequence 1
    And the buffered count is 0

  Scenario: Continuation without first frame is rejected
    When a continuation for key 99 with sequence 1 arrives
    Then the error is missing first frame for key 99

  Scenario: Duplicate first frame is rejected
    Given a first frame for key 1 with body "first"
    When the first frame is accepted
    And another first frame for key 1 with body "duplicate" arrives
    Then the error is duplicate first frame for key 1
    And the buffered count is 1

  Scenario: Message exceeding size limit is rejected
    Given a message assembly state with max size 10 and timeout 30 seconds
    And a first frame for key 1 with body "12345"
    When the first frame is accepted
    And a continuation for key 1 with body "exceeds limit" arrives
    Then the error is message too large for key 1
    And the buffered count is 0

  Scenario: Expired assemblies are purged
    Given a first frame for key 1 with body "partial"
    When the first frame is accepted at time T
    And time advances by 31 seconds
    And expired assemblies are purged
    Then key 1 was evicted
    And the buffered count is 0
