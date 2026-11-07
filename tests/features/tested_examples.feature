Feature: Published documentation examples
  User-facing examples should remain discoverable from the exact Markdown that
  readers receive.

  Scenario: Published examples are addressable by stable identifiers
    Given the published user-facing examples are loaded
    Then every published example has a stable identifier and source
