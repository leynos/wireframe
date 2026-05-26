Feature: Formal verification tooling metadata
  Contributors should be able to discover the pinned formal-verification tools
  and use Makefile entry points that delegate to rust-prover-tools.

  Scenario: The repository declares pinned prover tooling
    Given the formal verification tooling metadata is loaded
    Then the metadata pins Kani, Verus, and rust-prover-tools
    And the Verus checksum manifest covers the configured Linux archive
    And the Makefile exposes the formal verification tool entry points
    And the Makefile entry points delegate to prover-tools
