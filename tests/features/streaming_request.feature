@streaming_request
Feature: Streaming request body consumption
  Streaming request bodies let handlers consume inbound payloads
  incrementally while preserving back-pressure and error propagation.

  Scenario: StreamingBody exposes an AsyncRead adapter
    Given a streaming request body channel with capacity 4
    When body chunks "hello " and "world" are sent
    And the streaming body is read through the AsyncRead adapter
    Then the collected body is "hello world"

  Scenario: Bounded request body channels apply back-pressure
    Given a streaming request body channel with capacity 1
    When one body chunk "first" is buffered without draining the stream
    And another body chunk "second" is sent with a 50 millisecond timeout
    Then the send is blocked by back-pressure

  Scenario: Request body stream errors reach consumers
    Given a streaming request body channel with capacity 4
    When body chunk "ok" is sent
    And a request body error of kind "invalid data" is sent
    And the request body stream is drained directly
    Then one chunk is received before the error
    And the last stream error kind is "invalid data"
