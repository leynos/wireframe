@slow_io_backpressure
Feature: Slow reader and writer simulation
  Slow-I/O helpers pace the client write and read sides so back-pressure can be
  asserted deterministically under paused Tokio time.

  Scenario: Slow writer delays request completion
    Given a slow-io echo app with max frame length 4096
    When a 64-byte request is driven with slow writer pacing of 8 bytes every 5 milliseconds
    Then the slow-io drive remains pending
    When slow-io virtual time advances by 100 milliseconds
    Then the slow-io drive completes with an echoed payload of 64 bytes

  Scenario: Slow reader delays response draining
    Given a slow-io echo app with max frame length 4096
    When a slow reader drive is configured as 256/16/5/64
    Then the slow-io drive remains pending
    When slow-io virtual time advances by 200 milliseconds
    Then the slow-io drive completes with an echoed payload of 256 bytes

  Scenario: Combined slow reader and writer still round-trips correctly
    Given a slow-io echo app with max frame length 4096
    When a combined slow-io drive is configured as 96/12/5/24/5/64
    Then the slow-io drive remains pending
    When slow-io virtual time advances by 200 milliseconds
    Then the slow-io drive completes with an echoed payload of 96 bytes
