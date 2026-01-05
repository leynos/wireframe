Feature: Stateful codec payload wrapping
  Scenario: Sequence counters reset per connection
    Given a stateful wireframe server allowing frames up to 128 bytes
    When the first client sends 2 requests
    And the second client sends 1 request
    Then the first client observes sequence numbers 1 and 2
    And the second client observes sequence number 1
