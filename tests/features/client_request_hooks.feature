Feature: Client request hooks for outgoing and incoming frames
  The client supports before_send and after_receive hooks that fire on
  every outgoing request and incoming response frame, enabling symmetric
  instrumentation with the server middleware stack.

  Background:
    Given an envelope echo server for hook testing

  Scenario: Before-send hook is invoked for every outgoing frame
    Given a client with a before_send counter hook
    When the client sends an envelope via the hooked client
    Then the before_send counter is 1

  Scenario: After-receive hook is invoked for every incoming frame
    Given a client with an after_receive counter hook
    When the client sends and receives an envelope via the hooked client
    Then the after_receive counter is 1

  Scenario: Multiple hooks execute in registration order
    Given a client with two before_send hooks that append markers
    When the client sends an envelope via the hooked client
    Then the markers appear in registration order

  Scenario: Both hooks fire for a correlated call
    Given a client with both counter hooks
    When the client performs a correlated call via the hooked client
    Then the before_send counter is 1
    And the after_receive counter is 1

  Scenario: Before-send hook can mutate outbound frame bytes
    Given a client with a before_send hook that appends a marker byte
    When the client sends an envelope via the hooked client
    Then the captured frame ends with the marker byte

  Scenario: After-receive hook can mutate inbound bytes before deserialization
    Given a client with an after_receive hook that replaces the frame
    When the client sends and receives an envelope via the hooked client
    Then the received payload reflects the hook mutation
