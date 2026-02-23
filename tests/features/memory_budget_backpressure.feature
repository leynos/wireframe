@memory_budget_backpressure
Feature: Soft-limit memory budget back-pressure
  Inbound reads are paced when buffered assembly bytes approach configured
  aggregate memory budgets.

  Scenario: Soft pressure delays completion until virtual time advances
    Given a back-pressure inbound app configured as 200/2048/10/10
    When a budgeted first frame for key 1 with body "aaaaaaaa" arrives
    And a budgeted final continuation frame for key 1 sequence 1 with body "bb" arrives
    Then no budgeted payload is available before virtual time advances
    When budgeted virtual time advances by 5 milliseconds
    Then budgeted payload "aaaaaaaabb" is eventually received
    And no budgeted send error is recorded

  Scenario: Reads continue without delay when pressure is low
    Given a back-pressure inbound app configured as 200/2048/100/100
    When a budgeted first frame for key 2 with body "aa" arrives
    And a budgeted final continuation frame for key 2 sequence 1 with body "bb" arrives
    Then budgeted payload "aabb" is eventually received
    And no budgeted send error is recorded
