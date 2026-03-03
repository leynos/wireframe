@request_context
Feature: Tenant-aware request context
  The framework provides a cross-cutting request context carrying
  tenant, user, and tracing identifiers through the request pipeline.

  Scenario: Request context carries tenant identifier
    Given a request-context with tenant id 42
    When the request-context is extracted from the message request
    Then the extracted tenant id is 42

  Scenario: Request context carries all cross-cutting identifiers
    Given a request-context with tenant id 1 and user id 2 and correlation id 3
    When the request-context is extracted from the message request
    Then the extracted tenant id is 1
    And the extracted user id is 2
    And the extracted correlation id is 3

  Scenario: Missing tenant identifier fails require check
    Given a request-context without a tenant id
    When require-tenant-id is called on the request-context
    Then a missing tenant error is returned

  Scenario: Default context has no identifiers
    Given no request-context is registered
    When the request-context is extracted from the message request
    Then all extracted identifiers are absent
