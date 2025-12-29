Feature: Client connection lifecycle hooks
  The wireframe client supports lifecycle hooks that mirror the server's
  hook system, enabling consistent instrumentation across client and server.

  Scenario: Setup hook invoked on successful connection
    Given a standard echo server
    When a client connects with a setup callback
    Then the setup callback is invoked exactly once

  Scenario: Teardown hook invoked when connection closes
    Given a standard echo server
    When a client connects with setup and teardown callbacks
    And the client closes the connection
    Then the teardown callback is invoked exactly once
    And the teardown callback receives the state from setup

  Scenario: Error hook invoked on receive failure
    Given a standard echo server that disconnects immediately
    When a client connects with an error callback
    And the client attempts to receive a message
    Then the error callback is invoked
    And the client error is Disconnected

  Scenario: Lifecycle hooks work with preamble callbacks
    Given a preamble-aware echo server that sends acknowledgement
    When a client connects with preamble and lifecycle callbacks
    Then the preamble success callback is invoked
    And the setup callback is invoked after preamble exchange
