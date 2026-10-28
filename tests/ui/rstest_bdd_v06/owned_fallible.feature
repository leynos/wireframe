Feature: Fallible fixture signature compatibility
  Scenario: Owned fallible fixture is accepted
    Given a world from an owned fallible fixture
    Then the unwrapped world contains two
