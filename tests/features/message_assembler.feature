Feature: Message assembler header parsing
  Wireframe exposes a MessageAssembler hook for protocol-specific header
  parsing. The hook distinguishes first frames from continuation frames and
  exposes key metadata needed for assembly.

  Scenario: Builder exposes a configured message assembler
    Given a wireframe app with a message assembler
    And a first frame header with key 9 metadata length 2 body length 12
    When the message assembler parses the header
    Then the app exposes a message assembler
    And the parsed header is first
    And the message key is 9
    And the metadata length is 2
    And the body length is 12
    And the header length is 16
    And the total body length is absent
    And the frame is marked last false

  Scenario: Parsing a first frame header without total length
    Given a first frame header with key 9 metadata length 2 body length 12
    When the message assembler parses the header
    Then the parsed header is first
    And the message key is 9
    And the metadata length is 2
    And the body length is 12
    And the header length is 16
    And the total body length is absent
    And the frame is marked last false

  Scenario: Parsing a first frame header with total length
    Given a first frame header with key 42 body length 8 total 64
    When the message assembler parses the header
    Then the parsed header is first
    And the message key is 42
    And the metadata length is 0
    And the body length is 8
    And the header length is 20
    And the total body length is 64
    And the frame is marked last true

  Scenario: Parsing a continuation header with sequence
    Given a continuation header with key 7 body length 16 sequence 3
    When the message assembler parses the header
    Then the parsed header is continuation
    And the message key is 7
    And the body length is 16
    And the header length is 18
    And the sequence is 3
    And the frame is marked last false

  Scenario: Parsing a continuation header without sequence
    Given a continuation header with key 11 body length 5
    When the message assembler parses the header
    Then the parsed header is continuation
    And the message key is 11
    And the body length is 5
    And the header length is 14
    And the sequence is absent
    And the frame is marked last true

  Scenario: Invalid header payload returns error
    Given an invalid message header
    When the message assembler parses the header
    Then the parse fails with invalid data
