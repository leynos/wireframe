@client_streaming
Feature: Client streaming response consumption
  The client can consume multi-frame streaming responses (Response::Stream
  and Response::MultiPacket) by reading correlated frames until a
  protocol-defined end-of-stream terminator arrives.

  Background:
    Given a streaming echo server

  Scenario: Client receives a multi-frame streaming response
    When the client sends a streaming request with 3 data frames
    Then all 3 data frames are received in order
    And the stream terminates cleanly

  Scenario: Client receives an empty streaming response
    When the client sends a streaming request with 0 data frames
    Then no data frames are received
    And the stream terminates cleanly

  Scenario: Client detects correlation ID mismatch in stream
    Given a streaming server that returns mismatched correlation IDs
    When the client sends a streaming request
    Then a StreamCorrelationMismatch error is returned

  Scenario: Client handles server disconnect during stream
    Given a streaming server that disconnects after 2 frames
    When the client sends a streaming request
    Then 2 data frames are received
    And a disconnection error is returned

  Scenario: Client receives fair interleaving across push priorities
    Given a streaming server that emits interleaved high- and low-priority pushes
    When the client sends a streaming request
    Then interleaved priority frames are received without low-priority starvation
    And the stream terminates cleanly

  Scenario: Client observes shared rate limiting across push priorities
    Given a streaming server with shared cross-priority push rate limiting
    When the client sends a streaming request
    Then the shared limiter blocks cross-priority bursts before refill
    And the stream terminates cleanly
