# Documentation contents

This page summarises each file in the `docs/` directory.
Use it as a quick reference when navigating the project's design and
architecture material.

## Design documents

- [Outbound messaging design](asynchronous-outbound-messaging-design.md)  
  Comprehensive design for server-initiated pushes.
- [Message fragments](generic-message-fragmentation-and-re-assembly-design.md)  
  Design for fragmenting and reassembling messages.
- [Streaming response design](multi-packet-and-streaming-responses-design.md)  
  Design for handling multi-packet and streaming responses.
- [Router library design](rust-binary-router-library-design.md)  
  In-depth design of the binary router library.
- [Client design](wireframe-client-design.md)  
  Proposal for adding client-side support.
- [Frame metadata](frame-metadata.md)  
  How frame metadata assists with routing decisions.
- [Message versioning](message-versioning.md)  
  Approaches to message versioning and compatibility.
- [Preamble validator](preamble-validator.md)  
  Validating client connection preambles.

## Roadmaps

- [Outbound messaging roadmap](asynchronous-outbound-messaging-roadmap.md)  
  Task list for implementing asynchronous outbound messaging.
- [Wireframe 1.0 roadmap](wireframe-1-0-detailed-development-roadmap.md)  
  Detailed tasks leading to Wireframe 1.0.
- [Project roadmap](roadmap.md)  
  High-level development roadmap.
- [1.0 philosophy](the-road-to-wireframe-1-0-feature-set-philosophy-and-capability-maturity.md)  
  Philosophy and feature set for Wireframe 1.0.

## Testing

- [Multi-layered testing strategy](multi-layered-testing-strategy.md)  
  Overview of the library's testing approach.
- [RSTest fixtures](rust-testing-with-rstest-fixtures.md)  
  Using `rstest` fixtures for clean tests.
- [Mocking network outages](mocking-network-outages-in-rust.md)  
  Tutorial for simulating network outages in tests.
- [Testing helpers](wireframe-testing-crate.md)  
  Planned companion crate with testing helpers.

## Operations and resilience

- [Resilience guide](hardening-wireframe-a-guide-to-production-resilience.md)  
  Guidance on achieving production resilience.
- [Observability and operability](observability-operability-and-maturity.md)  
  Guide to observability and operational maturity.

## Reference guides

- [Refactoring guide](complexity-antipatterns-and-refactoring-strategies.md)  
  Strategies for taming code complexity and refactoring.
- [Documentation style guide](documentation-style-guide.md)  
  Conventions for writing project documentation.

