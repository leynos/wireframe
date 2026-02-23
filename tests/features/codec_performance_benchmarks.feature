Feature: Codec performance benchmark coverage
  Scenario: Encode and decode matrix covers default and custom codecs
    Given codec benchmark validation uses 16 iterations
    When encode and decode benchmark samples run across the codec matrix
    Then encode and decode benchmark samples report all workloads

  Scenario: Fragmentation overhead sample is recorded
    Given codec benchmark validation uses 16 iterations
    When fragmented and unfragmented wrapping samples are executed
    Then fragmentation overhead metrics are available

  Scenario: Allocation baseline labels include wrap and decode counters
    Given codec benchmark validation uses 16 iterations
    When allocation baseline labels are generated for codec workloads
    Then allocation baseline labels include wrap and decode counters
