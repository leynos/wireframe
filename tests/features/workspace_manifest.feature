Feature: Hybrid workspace manifest
  The repository should expose an explicit hybrid Cargo workspace while keeping
  the root package as the only default member during roadmap item 10.1.1.

  Scenario: The root manifest stages the hybrid workspace conversion
    Given the repository workspace metadata is loaded
    Then the root Cargo manifest declares the staged hybrid workspace
    And the workspace metadata reports the root package as a workspace member
    And the workspace metadata reports the root package as the only default member
    And the workspace metadata does not include the verification crate yet
