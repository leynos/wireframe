@memory_budgets
Feature: Memory budgets builder configuration
  Wireframe applications can define explicit per-connection memory budgets for
  inbound buffering and assembly.

  Scenario: Create memory budgets with explicit byte caps
    Given memory budgets with message bytes 1024, connection bytes 4096, and in-flight bytes 2048
    Then the message budget is 1024 bytes
    And the connection budget is 4096 bytes
    And the in-flight budget is 2048 bytes

  Scenario: Configure a Wireframe app with memory budgets
    Given memory budgets with message bytes 1024, connection bytes 4096, and in-flight bytes 2048
    When configuring a Wireframe app with memory budgets
    Then app configuration with memory budgets succeeds

  Scenario: Compose memory budgets with codec configuration
    Given memory budgets with message bytes 1024, connection bytes 4096, and in-flight bytes 2048
    When configuring a Wireframe app with memory budgets and a custom codec budget
    Then app configuration with memory budgets succeeds
