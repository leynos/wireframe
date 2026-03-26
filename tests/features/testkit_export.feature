@testkit-export
Feature: `wireframe::testkit` public export

  Scenario: Chunked partial-frame driving is available from the root crate
    Given a wireframe app with a Hotline codec allowing 4096-byte frames for testkit export
    When a payload is driven through `wireframe::testkit` in 2-byte chunks
    Then the testkit export returns a non-empty response payload

  Scenario: Reassembly assertions are available from the root crate
    Given a completed message-assembly snapshot for key 7 in the testkit export world
    When the snapshot is asserted through `wireframe::testkit`
    Then the testkit reassembly assertion succeeds
