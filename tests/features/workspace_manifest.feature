Feature: Formal verification workspace manifest
  The repository should expose an explicit hybrid Cargo workspace while keeping
  the root package as the only default member after adding the verification
  crate in roadmap item 15.1.2.

  Scenario: The root manifest adds the verification crate without widening defaults
    Given the repository workspace metadata is loaded
    Then the root Cargo manifest declares the staged hybrid workspace
    And the workspace metadata reports the root package as a workspace member
    And the workspace metadata reports the root package as the only default member
    And the workspace metadata includes the verification crate as a workspace member
