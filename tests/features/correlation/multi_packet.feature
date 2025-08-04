Feature: Multi-packet correlation ids
  Scenario: Response frames carry request correlation id
    Given a multi-packet response stream with correlation id 7
    When the connection actor runs to completion
    Then each emitted frame has correlation id 7
