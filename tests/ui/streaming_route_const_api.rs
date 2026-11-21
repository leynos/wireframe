//! Accept a streaming route handler whose future is `Send`.

use wireframe::{
    app::{StreamingHandler, WireframeApp},
    prelude::BincodeSerializer,
    request::{RequestBodyStream, RequestParts},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let handler: StreamingHandler = std::sync::Arc::new(
        |_parts: RequestParts, body: RequestBodyStream| {
            Box::pin(async move {
                // Hold the body across the await so the future really does
                // capture it, as a handler that reads its body would.
                let _body = body;
                std::future::ready(()).await;
                None
            })
        },
    );
    let _app = WireframeApp::<BincodeSerializer>::new()?.streaming_route(7, handler)?;
    Ok(())
}
