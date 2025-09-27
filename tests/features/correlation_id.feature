Feature: Multi-packet response correlation
  Scenario: Streamed frames reuse the request correlation id
    Given a correlation id 7
    When a stream of frames is processed
    Then each emitted frame uses correlation id 7

  Scenario: Multi-packet responses reuse the request correlation id
    Given a correlation id 11
    When a multi-packet channel emits frames
    Then each emitted frame uses correlation id 11
