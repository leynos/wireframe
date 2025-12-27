@request_parts
Feature: Request parts metadata handling
  RequestParts separates routing metadata from streaming request payloads,
  enabling handlers to begin processing before the full body arrives.

  Scenario: Create request parts with all fields
    Given request parts with id 42 and correlation id 100
    And metadata bytes 1, 2, 3
    Then the request id is 42
    And the correlation id is 100
    And the metadata length is 3

  Scenario: Request parts inherit missing correlation id
    Given request parts with id 1 and no correlation id
    When inheriting correlation id 55
    Then the correlation id is 55

  Scenario: Request parts override mismatched correlation id
    Given request parts with id 1 and correlation id 7
    When inheriting correlation id 8
    Then the correlation id is 8

  Scenario: Request parts preserve correlation when source is absent
    Given request parts with id 1 and correlation id 42
    When inheriting no correlation id
    Then the correlation id is 42

  Scenario: Empty metadata is valid
    Given request parts with id 1, no correlation id, and empty metadata
    Then the metadata length is 0

  Scenario: Metadata can be modified after construction
    Given request parts with id 1 and no correlation id
    And metadata byte 1
    When appending byte 2 to metadata
    Then the metadata length is 2
