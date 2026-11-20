@zero_copy_baseline
Feature: Zero-copy baseline gate
  The baseline claims that read-only stages never copy the payload and that
  inbound decode copies it exactly once. These scenarios state those claims as
  observable behaviour. The allocation-level half of the read-only claim is
  enforced by the harness-free invariants binary, because a counting allocator
  cannot be installed for one scenario without instrumenting the whole suite.

  Scenario: A pass-through middleware leaves the payload in place
    Given a default-path envelope carrying a large payload
    When the envelope passes through a read-only middleware chain
    Then the payload buffer is the one the envelope arrived with
    And the read-only chain leaves the payload unchanged

  Scenario: A copying middleware is detected
    Given a default-path envelope carrying a large payload
    When the envelope passes through a middleware that copies its frame
    Then the payload buffer differs from the one the envelope arrived with

  Scenario: Inbound decode copies the payload exactly once
    Given a framed default-path message carrying a large payload
    When the message is decoded into an envelope
    Then the deserialized copied-byte count equals the payload length