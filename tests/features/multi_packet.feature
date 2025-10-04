@multi_packet
Feature: Multi-packet responses
  Scenario: Response::with_channel streams frames sequentially
    When a handler uses the with_channel helper to emit messages
    Then all messages are received in order

  Scenario: no messages are emitted from a multi-packet response
    When a handler uses the with_channel helper to emit no messages
    Then no messages are received
