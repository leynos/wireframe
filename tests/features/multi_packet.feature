Feature: Multi-packet responses
  Scenario: messages from a multi-packet response are delivered sequentially
    When a multi-packet response emits messages
    Then all messages are received in order

  Scenario: no messages are emitted from a multi-packet response
    When a multi-packet response emits no messages
    Then no messages are received
