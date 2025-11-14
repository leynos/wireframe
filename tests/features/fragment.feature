@fragment
Feature: Fragment metadata enforcement
  Scenario: Sequential fragments complete a message
    Given a fragment series for message 41
    When fragment 0 arrives marked non-final
    And fragment 1 arrives marked final
    Then the fragment completes the message

  Scenario: Out-of-order fragment is rejected
    Given a fragment series for message 17
    When fragment 1 arrives marked non-final
    Then the fragment is rejected as out-of-order

  Scenario: Fragment from another message is rejected
    Given a fragment series for message 5
    When fragment 0 from message 6 arrives marked non-final
    Then the fragment is rejected for the wrong message
