@client-pair-harness
Feature: In-process server and client pair harness

  Scenario: Downstream crate verifies a request/response contract through the pair harness
    Given a server/client pair running an echo app via the client pair harness
    When a request with id 1 and correlation 7 is sent through the client pair harness
    Then the client pair harness response has correlation id 7 and matching payload

  Scenario: Downstream crate supplies non-default client configuration through the pair harness
    Given a server/client pair with max frame length 2048 via the client pair harness
    When a request with id 1 and correlation 42 is sent through the client pair harness
    Then the client pair harness response has correlation id 42 and matching payload
