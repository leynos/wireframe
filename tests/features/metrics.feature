Feature: metrics exporter
  Scenario: active connections metric recorded
    Given a metrics recorder
    When a connection runs with no frames
    Then the exporter output includes active connections gauge
