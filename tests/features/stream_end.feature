Feature: Stream terminator frame
  Scenario: Connection actor emits terminator after stream
    When a streaming response completes
    Then an end-of-stream frame is sent

  Scenario: Multi-packet channel emits terminator after completion
    When a multi-packet channel drains
    Then a multi-packet end-of-stream frame is sent
