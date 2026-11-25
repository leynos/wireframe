@server-harness-lifecycle
Feature: Protocol-agnostic server lifecycle harness

  Scenario: A configured server becomes ready and accepts a connection
    Given a configured Wireframe echo server that has not been bound
    When the server lifecycle harness starts the server
    Then the harness reports a loopback address
    And a client can open a connection to that address

  Scenario: Stopping the harness twice is safe
    Given a configured Wireframe echo server that has not been bound
    When the server lifecycle harness starts the server
    And the harness is stopped twice
    Then both stop attempts succeed
    And the harness reports that it completed
    And the address refuses new connections

  Scenario: Dropping the harness without stopping it releases the port
    Given a configured Wireframe echo server that has not been bound
    When the server lifecycle harness starts the server
    And the harness is dropped without being stopped
    Then the address stops accepting connections within one second

  Scenario: A failing application factory is reported at startup
    Given a configured Wireframe server whose application factory fails
    When the server lifecycle harness starts the server
    Then starting the server fails with an application startup error