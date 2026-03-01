@client_send_streaming
Feature: Client outbound streaming sends
  The client can send large request bodies as multiple frames by reading
  from an AsyncRead source with protocol-provided frame headers.

  Background:
    Given a send-streaming receiving server

  Scenario: Client sends a multi-chunk body
    When the client streams 300 bytes with a 4 byte header and 100 byte chunks
    Then the send-streaming server receives 3 frames
    And each send-streaming frame starts with the protocol header

  Scenario: Client sends an empty body
    When the client streams 0 bytes with a 4 byte header and 100 byte chunks
    Then the send-streaming server receives 0 frames

  Scenario: Client send operation times out on a slow body reader
    Given a send-streaming body reader that blocks indefinitely
    When the client streams with a 100 ms timeout
    Then a send-streaming TimedOut error is returned

  Scenario: Client handles transport failure during streaming send
    Given a send-streaming server that disconnects immediately
    When the client streams 300 bytes with a 4 byte header and 100 byte chunks
    Then a send-streaming transport error is returned
