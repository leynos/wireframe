@interleaved
Feature: Interleaved high- and low-priority push queues
  Scenario: High-priority frames take precedence when fairness is disabled
    When both queues carry traffic with fairness disabled
    Then all high-priority frames precede all low-priority frames

  Scenario: Fairness yields to low-priority after burst threshold
    When both queues carry traffic with a fairness burst threshold
    Then low-priority frames are interleaved at the configured interval

  Scenario: Rate limiting applies symmetrically across both priority levels
    When a high-priority push exhausts the rate limit
    Then a low-priority push is also blocked until the next window

  Scenario: All frames are delivered when both queues carry traffic
    When both queues carry interleaved traffic with fairness enabled
    Then every enqueued frame is present in the output
