Feature: Client structured logging and tracing spans
  The wireframe client emits tracing spans and structured events around
  connect, send, receive, call, streaming, and close operations.
  Per-command timing can be enabled via TracingConfig.

  Scenario: Connect emits a tracing span with the peer address
    Given a running echo server for tracing tests
    And a client with connect timing enabled
    When the client connects to the server
    Then the tracing output contains "client.connect"
    And the tracing output contains the peer address

  Scenario: Send emits a tracing span with frame size
    Given a connected tracing client with send timing enabled
    When the client sends an envelope via the tracing client
    Then the tracing output contains "client.send"
    And the tracing output contains "frame.bytes"

  Scenario: Receive emits a tracing span recording result
    Given a connected tracing client with receive timing enabled
    When the client sends and receives via the tracing client
    Then the tracing output contains "client.receive"

  Scenario: Per-command timing emits elapsed microseconds
    Given a connected tracing client with send timing enabled
    When the client sends an envelope via the tracing client
    Then the tracing output contains "elapsed_us"

  Scenario: Timing is not emitted when disabled
    Given a connected tracing client with default config
    When the client sends an envelope via the tracing client
    Then the tracing output does not contain "elapsed_us"

  Scenario: Close emits a tracing span
    Given a connected tracing client with close timing enabled
    When the tracing client closes the connection
    Then the tracing output contains "client.close"
