Feature: Wireframe client runtime
  Scenario: Client sends and receives with configured frame length
    Given a wireframe echo server allowing frames up to 2048 bytes
    And a wireframe client configured with max frame length 2048
    When the client sends a payload of 1500 bytes
    Then the client receives the echoed payload

  Scenario: Client reports errors when server frame limit is exceeded
    Given a wireframe echo server allowing frames up to 64 bytes
    And a wireframe client configured with max frame length 1024
    When the client sends an oversized payload of 128 bytes
    Then the client reports a Wireframe transport error

  Scenario: Client maps malformed responses to decode protocol errors
    Given a wireframe server that replies with malformed payloads
    And a wireframe client configured with max frame length 1024
    When the client sends a payload of 128 bytes expecting decode failure
    Then the client reports a Wireframe decode protocol error

  Scenario: Client decodes echoed login acknowledgement
    Given a wireframe echo server allowing frames up to 2048 bytes
    And a wireframe client configured with max frame length 2048
    When the client sends a login request for username "guest"
    Then the client decodes a login acknowledgement for username "guest"
