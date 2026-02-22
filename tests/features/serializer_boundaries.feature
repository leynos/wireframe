@serializer_boundaries
Feature: Serializer boundaries and metadata context
  Serializer boundaries should remain compatible with legacy bincode messages
  while exposing metadata context to deserializers.

  Scenario: Legacy message round-trips through serializer-agnostic adapters
    Given a legacy payload value 7
    When the legacy payload is encoded and decoded
    Then the decoded legacy payload value is 7

  Scenario: Metadata context is forwarded to deserialization
    Given deserialize context message id 9 and correlation id 77
    When a context-aware serializer decodes with context
    Then the captured message id is 9
    And the captured correlation id is 77
