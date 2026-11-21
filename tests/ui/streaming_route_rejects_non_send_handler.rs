//! Reject a streaming route handler that holds a non-`Send` value across an await.

use std::{future, rc::Rc};

use wireframe::{
    app::{StreamingHandler, WireframeApp},
    prelude::BincodeSerializer,
    request::{RequestBodyStream, RequestParts},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let handler: StreamingHandler = std::sync::Arc::new(
        |_parts: RequestParts, body: RequestBodyStream| {
            Box::pin(async move {
                let shared = Rc::new(());
                let _body = body;
                future::ready(()).await;
                let _keep = Rc::clone(&shared);
                None
            })
        },
    );
    let _app = WireframeApp::<BincodeSerializer>::new()?.streaming_route(7, handler)?;
    Ok(())
}
