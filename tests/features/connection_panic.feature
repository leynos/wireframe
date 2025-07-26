Feature: Connection panic resilience
  Scenario: connection panic does not crash server
    Given a running wireframe server with a panic in connection setup
    When I connect to the server
    And I connect to the server again
    Then both connections succeed
