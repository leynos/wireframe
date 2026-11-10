Feature: Published documentation examples
  User-facing examples should remain discoverable from the exact Markdown that
  readers receive, and a document that breaks the marker rules should be
  rejected before it ships.

  Scenario: Published examples are addressable by stable identifiers
    Given the published user-facing examples are loaded
    Then every published example has a stable identifier and source

  Scenario: A well-formed example is accepted by the shared loader
    Given a document whose example carries a marker and a language
    When the shared loader validates the staged document
    Then validation succeeds

  Scenario: A malformed example is rejected by the shared loader
    Given a document whose example fence carries no language
    When the shared loader validates the staged document
    Then validation fails and names the fence line
