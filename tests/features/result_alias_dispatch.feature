Feature: Aliased step results
  Result aliases must preserve error propagation and fixture overrides.

  Scenario: An aliased step error fails the scenario
    When an aliased step returns an error

  Scenario: An aliased successful value overrides its fixture
    When an aliased step returns the replacement value
    Then the replacement fixture is visible as 2
