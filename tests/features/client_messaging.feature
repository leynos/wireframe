Feature: Client message API with correlation identifiers
  The client provides async send, receive, and call APIs that encode Message
  implementers, forward correlation identifiers, and deserialize typed responses.

  Background:
    Given an envelope echo server

  Scenario: Client auto-generates correlation ID when sending envelope
    Given an envelope without a correlation ID
    When the client sends the envelope
    Then the envelope is stamped with an auto-generated correlation ID

  Scenario: Client preserves explicit correlation ID when sending envelope
    Given an envelope with correlation ID 42
    When the client sends the envelope
    Then the returned correlation ID is 42

  Scenario: Client call_correlated validates response correlation ID
    Given an envelope without a correlation ID
    When the client calls the server with call_correlated
    Then the response has a matching correlation ID
    And no correlation mismatch error occurs

  Scenario: Client detects correlation ID mismatch
    Given an envelope without a correlation ID
    And a server that returns mismatched correlation IDs
    When the client calls the server with call_correlated
    Then a CorrelationMismatch error is returned

  Scenario: Client generates unique correlation IDs for sequential requests
    When the client sends 5 sequential envelopes
    Then each envelope has a unique correlation ID

  Scenario: Client round-trips multiple message types
    Given an envelope with message ID 1 and payload "hello"
    When the client calls the server with call_correlated
    Then the response contains the same message ID and payload
