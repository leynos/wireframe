Feature: Client preamble exchange
  The wireframe client can send a preamble before exchanging frames.

  Scenario: Client sends preamble and server acknowledges
    Given a preamble-aware echo server
    When a client connects with a preamble containing version 42
    Then the server receives the preamble with version 42
    And the client success callback is invoked

  Scenario: Client receives server acknowledgement in success callback
    Given a preamble-aware echo server that sends an acknowledgement preamble
    When a client connects with a preamble and reads the acknowledgement
    Then the client receives an accepted acknowledgement

  Scenario: Client preamble timeout triggers failure callback
    Given a slow preamble server that never responds
    When a client connects with a 50ms preamble timeout
    Then the client fails with a timeout error
    And the failure callback is invoked

  Scenario: Client without preamble connects normally
    Given a standard echo server without preamble support
    When a client connects without a preamble
    Then the client connects successfully
