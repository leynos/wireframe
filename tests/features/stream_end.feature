Feature: Stream terminator frame
  Scenario: Connection actor emits terminator after stream
    When a streaming response completes
    Then an end-of-stream frame is sent
