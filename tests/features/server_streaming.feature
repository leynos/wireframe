@server_streaming
Feature: Server streams responses through the connection actor
  Handlers registered with route_response return explicit responses, and the
  connection actor streams those responses to the peer. Every data frame
  inherits the request correlation identifier, and the protocol terminator
  closes the stream.

  Background:
    Given a server whose routes stream responses

  Scenario: Server streams an ordered response
    When the client requests the streamed response
    Then the server sends 3 data frames
    And the data frames arrive in ascending order
    And the streamed response terminates cleanly

  Scenario: Server streams a spawned multi-packet response
    When the client requests the multi-packet response
    Then the server sends 2 data frames
    And the data frames arrive in ascending order
    And the streamed response terminates cleanly

  Scenario: Server streams an empty response
    When the client requests the empty stream response
    Then the server sends 0 data frames
    And the streamed response terminates cleanly
