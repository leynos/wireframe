Feature: Streaming request bodies reach route handlers

  A route registered with `streaming_route` receives its request body while
  the message's frames are still arriving. Buffered routes keep their existing
  reassembly guarantees, and a streaming body's failure must never leave the
  connection unable to dispatch the messages that follow it.

  Scenario: A streaming body is delivered while its frames arrive
    Given an app with a streaming route and a buffered route
    When a streaming first frame for key 7 with body "alpha" arrives
    Then the handler receives chunk "alpha"
    And the handler is still reading its body
    When a final streaming continuation frame for key 7 sequence 1 with body "omega" arrives
    Then the handler receives chunk "omega"
    And the handler observes the body completed
    And the reply payload is "stream-reply"
    And the connection ends without error

  Scenario: A complete buffered message keeps its reassembly guarantees
    Given an app with a streaming route and a buffered route
    When a complete buffered message for key 8 with body "beta" arrives
    Then the buffered handler receives payload "beta"
    And the reply payload is "beta"
    And the connection ends without error

  Scenario: A paused frame header resumes without losing the message
    Given an app with a streaming route and a buffered route
    When the first part of a complete buffered message for key 9 with body "delta" arrives
    Then the buffered handler receives no payload
    When the rest of the buffered message arrives
    Then the buffered handler receives payload "delta"
    And the reply payload is "delta"
    And the connection ends without error

  Scenario: A held consumer parks later messages and replies in order
    Given an app whose streaming handler holds its body
    When a streaming first frame for key 10 with body "one" arrives
    And a final streaming continuation frame for key 10 sequence 1 with body "two" arrives
    And a non-final buffered frame for key 11 with body "three" arrives
    And a final buffered continuation frame for key 11 sequence 1 with body "four" arrives
    Then the buffered handler receives no payload
    When the handler is released
    Then the handler receives chunk "one"
    And the handler receives chunk "two"
    And the handler observes the body completed
    And the reply payload is "stream-reply"
    And the buffered handler receives payload "threefour"
    And the reply payload is "threefour"

  Scenario: An abandoned body does not block the next message
    Given an app whose streaming handler abandons its body
    When a streaming first frame for key 12 with body "ignored" arrives
    Then the reply payload is "abandoned"
    When a complete buffered message for key 13 with body "next" arrives
    Then the buffered handler receives payload "next"
    And the reply payload is "next"
    And the connection ends without error

  Scenario: A parked streaming message does not hold the connection open
    Given an app whose streaming handler holds its body
    When a streaming first frame for key 18 with body "held" arrives
    And a streaming first frame for key 19 with body "late" arrives
    And a final streaming continuation frame for key 19 sequence 1 with body "st" arrives
    And the connection closes
    Then the connection ends without error

  Scenario: End of stream reports an unexpected end to the body
    Given an app with a streaming route and a buffered route
    When a streaming first frame for key 14 with body "partial" arrives
    Then the handler receives chunk "partial"
    When the connection closes
    Then the handler observes body error "unexpected end of file"

  Scenario: A declared total overshoot fails the body
    Given an app with a streaming route and a buffered route
    When a streaming first frame for key 15 with body "ab" and declared total 4 arrives
    Then the handler receives chunk "ab"
    When a final streaming continuation frame for key 15 sequence 1 with body "cdef" arrives
    Then the handler observes body error "invalid data"
    And the reply payload is "stream-reply"

  Scenario: A short final frame fails the body
    Given an app with a streaming route and a buffered route
    When a streaming first frame for key 16 with body "abc" and declared total 5 arrives
    Then the handler receives chunk "abc"
    When a final streaming continuation frame for key 16 sequence 1 with body "d" arrives
    Then the handler observes body error "invalid data"
    And the reply payload is "stream-reply"

  Scenario: An idle body expires and reports a timeout
    Given an app with a streaming route and a 50 millisecond assembly timeout
    When a streaming first frame for key 17 with body "stalled" arrives
    Then the handler receives chunk "stalled"
    When time passes for 200 milliseconds
    Then the handler observes body error "timed out"