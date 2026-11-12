Feature: Published documentation examples
  User-facing examples should remain discoverable from the exact Markdown that
  readers receive, and a document that breaks the marker rules should be
  rejected before it ships.

  Scenario: Published examples are addressable by stable identifiers
    Given the published user-facing examples are loaded
    Then every published example has a stable identifier and source

  Scenario: The shipped documentation passes the loader gate
    Given the shipped documentation is read from disk
    When the shared loader validates the shipped documentation
    Then validation succeeds

  Scenario: A shipped document that loses a marker fails the loader gate
    Given a shipped guide has lost one example marker
    When the shared loader validates the shipped documentation
    Then validation fails and names the document and the missing marker
