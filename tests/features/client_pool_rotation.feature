Feature: Client pool slot rotation
  The client pool rotates the first slot it attempts so consecutive leases
  spread across physical sockets, skips a slot that has no spare capacity, and
  resumes a waiter that was parked while every slot was leased. Shutdown and
  error paths are covered by the pool unit tests; these scenarios pin the
  pool-level policy through the public API.

  Scenario: Client pool rotates slots and resumes a parked acquisition
    Given client pool rotation is configured with two single-permit slots
    When client pool acquires two leases and parks a third acquisition
    Then client pool leases rotate across both slots
    And client pool resumes the parked acquisition on the released slot
    And client pool observes one physical connection per slot

  Scenario: Client pool skips a leased starting slot when another slot is free
    Given client pool rotation is configured with two single-permit slots
    When client pool keeps one lease and releases the other
    Then client pool reacquires from the released slot
    And client pool leaves the kept lease usable
