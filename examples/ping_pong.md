# Ping-Pong Example

This example demonstrates routing, serialization, and middleware usage in a
small ping/pong protocol. The server accepts a `Ping` message containing a
counter and responds with a `Pong` containing the incremented value. Logging
middleware prints each request and response.

```mermaid
classDiagram
    class Ping {
        +u32 0
        +to_bytes()
        +from_bytes()
    }
    class Pong {
        +u32 0
        +to_bytes()
        +from_bytes()
    }
    class ErrorMsg {
        +String 0
        +to_bytes()
        +from_bytes()
    }
    class PongMiddleware {
    }
    class PongService {
        +inner: S
        +call(req: ServiceRequest) Result<ServiceResponse, Infallible>
    }
    class Logging {
    }
    class LoggingService {
        +inner: S
        +call(req: ServiceRequest) Result<ServiceResponse, Infallible>
    }
    class HandlerService {
        +id()
        +from_service(id, service)
    }
    PongMiddleware --|> Transform
    PongService --|> Service
    Logging --|> Transform
    LoggingService --|> Service
    HandlerService <.. PongService : inner
    HandlerService <.. LoggingService : inner
    PongMiddleware ..> HandlerService : transform
    Logging ..> HandlerService : transform
    WireframeApp <.. build_app : factory
    WireframeServer <.. main : uses
    build_app --> WireframeApp
    main --> WireframeServer
```
