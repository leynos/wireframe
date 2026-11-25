@zero_copy_baseline
Feature: Zero-copy benchmark harness baseline
  These scenarios pin the benchmark harness the zero-copy gate measures: the
  read-only chain built from the library's own `Transform` and `Service`
  traits, its seeded copying fault, and the decode seam the copied-byte counter
  observes. They are harness-level checks, not application-boundary tests; the
  end-to-end application path, including `forward_response`, is roadmap item
  `13.2.2`.

  The allocation-level half of the read-only claim is enforced by the
  harness-free invariants binary, because a counting allocator cannot be
  installed for one scenario without instrumenting the whole suite. The
  copied-byte counts are asserted in `wireframe_testing/tests/copied_bytes.rs`:
  those counters are process-global, so reading them here would race every
  other scenario in this binary that decodes a message.

  Scenario: The harness read-only chain leaves the payload in place
    Given a default-path envelope carrying a large payload
    When the envelope passes through a read-only middleware chain
    Then the payload buffer is the one the envelope arrived with
    And the read-only chain leaves the payload unchanged

  Scenario: The harness copying chain is detected
    Given a default-path envelope carrying a large payload
    When the envelope passes through a middleware that copies its frame
    Then the payload buffer differs from the one the envelope arrived with

  Scenario: The harness decode seam returns the framed payload
    Given a framed default-path message carrying a large payload
    When the message is decoded through the benchmark decode seam
    Then the decoded payload is the payload the frame carried
    And the whole frame is consumed