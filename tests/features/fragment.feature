@fragment
Feature: Fragment metadata enforcement
  Scenario: Sequential fragments complete a message
    Given a fragment series for message 41
    When fragment 0 arrives marked non-final
    And fragment 1 arrives marked final
    Then the fragment completes the message

  Scenario: Out-of-order fragment is rejected
    Given a fragment series for message 17
    When fragment 1 arrives marked non-final
    Then the fragment is rejected as out-of-order

  Scenario: Fragment from another message is rejected
    Given a fragment series for message 5
    When fragment 0 from message 6 arrives marked non-final
    Then the fragment is rejected for the wrong message

  Scenario: Fragment beyond the maximum index is rejected
    Given a fragment series for message 9
    And the series expects fragment index 4294967295
    When fragment 4294967295 arrives marked non-final
    Then the fragment is rejected for index overflow

  Scenario: Final fragment at the maximum index completes the message
    Given a fragment series for message 10
    And the series expects fragment index 4294967295
    When fragment 4294967295 arrives marked final
    Then the fragment completes the message

  Scenario: Series rejects fragments after completion
    Given a fragment series for message 11
    When fragment 0 arrives marked final
    And fragment 1 arrives marked final
    Then the fragment is rejected because the series is complete

  Scenario: Fragmenter splits oversized payloads into sequential fragments
    Given a fragmenter capped at 3 bytes per fragment
    When the fragmenter splits a payload of 8 bytes
    Then the fragmenter produces 3 fragments
    And fragment 0 carries 3 bytes
    And fragment 0 is marked non-final
    And fragment 1 carries 3 bytes
    And fragment 1 is marked non-final
    And fragment 2 carries 2 bytes
    And fragment 2 is marked final
    And the fragments use message id 0

  Scenario: Reassembler rebuilds sequential fragments
    Given a reassembler allowing 10 bytes with a 30-second reassembly timeout
    When fragment 0 for message 21 with 4 bytes arrives marked non-final
    And fragment 1 for message 21 with 3 bytes arrives marked final
    Then the reassembler outputs a payload of 7 bytes
    And the reassembler is buffering 0 messages

  Scenario: Reassembler rejects messages that exceed the cap
    Given a reassembler allowing 4 bytes with a 30-second reassembly timeout
    When fragment 0 for message 22 with 3 bytes arrives marked non-final
    And fragment 1 for message 22 with 2 bytes arrives marked final
    Then the reassembler reports a message-too-large error
    And no message has been reassembled yet
    And the reassembler is buffering 0 messages

  Scenario: Reassembler evicts stale partial messages
    Given a reassembler allowing 8 bytes with a 1-second reassembly timeout
    When fragment 0 for message 23 with 5 bytes arrives marked non-final
    And time advances by 2 seconds for reassembly
    And expired reassembly buffers are purged
    Then the reassembler is buffering 0 messages
    And message 23 is evicted
    And no message has been reassembled yet

  Scenario: Reassembler rejects out-of-order fragments
    Given a reassembler allowing 8 bytes with a 30-second reassembly timeout
    When fragment 1 for message 24 with 2 bytes arrives marked non-final
    Then the reassembler reports an out-of-order fragment error
    And the reassembler is buffering 0 messages

  Scenario: Reassembler suppresses duplicate fragments
    Given a reassembler allowing 8 bytes with a 30-second reassembly timeout
    When fragment 0 for message 25 with 2 bytes arrives marked non-final
    And fragment 0 for message 25 with 2 bytes arrives marked non-final
    Then no message has been reassembled yet
    And the reassembler is buffering 1 message
    When fragment 1 for message 25 with 1 byte arrives marked final
    Then the reassembler outputs a payload of 3 bytes
    And the reassembler is buffering 0 messages

  Scenario: Reassembler handles zero-length fragments
    Given a reassembler allowing 8 bytes with a 30-second reassembly timeout
    When fragment 0 for message 26 with 0 bytes arrives marked final
    Then the reassembler outputs a payload of 0 bytes
    And the reassembler is buffering 0 messages

  Scenario: Reassembler rebuilds interleaved messages
    Given a reassembler allowing 12 bytes with a 30-second reassembly timeout
    When fragment 0 for message 27 with 3 bytes arrives marked non-final
    And fragment 0 for message 28 with 4 bytes arrives marked non-final
    And fragment 1 for message 27 with 2 bytes arrives marked final
    Then the reassembler outputs a payload of 5 bytes
    And the reassembler is buffering 1 message
    When fragment 1 for message 28 with 1 byte arrives marked final
    Then the reassembler outputs a payload of 5 bytes
    And the reassembler is buffering 0 messages
