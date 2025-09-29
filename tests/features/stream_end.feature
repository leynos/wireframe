Feature: Stream terminator frame
  Scenario: Connection actor emits terminator after stream
    When a streaming response completes
    Then an end-of-stream frame is sent

  Scenario: Multi-packet channel emits terminator after completion
    When a multi-packet channel drains
    Then a multi-packet end-of-stream frame is sent

  Scenario: Multi-packet channel disconnect logs termination
    When a multi-packet channel disconnects abruptly
    Then a multi-packet end-of-stream frame is sent
    And the multi-packet termination reason is disconnected

  Scenario: Shutdown closes a multi-packet channel
    When shutdown closes a multi-packet channel
    Then no multi-packet terminator is sent
    And the multi-packet termination reason is shutdown
