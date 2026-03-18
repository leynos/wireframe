Feature: Client pool handles
  Pool handles represent logical sessions so the pooled client can order
  blocked acquisition fairly without bypassing back-pressure.

  Scenario: Two logical sessions alternate fairly on one pooled socket
    Given client pool handle round-robin fairness is configured
    When two logical sessions repeatedly acquire one pooled socket
    Then the logical sessions alternate fairly

  Scenario: FIFO fairness preserves waiting order
    Given client pool handle FIFO fairness is configured
    When three logical sessions wait in order on one pooled socket
    Then the logical sessions are served in arrival order

  Scenario: Waiting handle remains blocked until a lease is released
    Given client pool handle back-pressure is configured
    When one logical session holds the only pooled lease
    Then another logical session stays blocked until capacity returns

  Scenario: Handle warm reuse and idle recycle match pool behaviour
    Given client pool handle warm reuse and idle recycle are configured
    When one logical session reuses a warm socket and then idles past timeout
    Then the handle preserves warm reuse and later reconnects after idle
