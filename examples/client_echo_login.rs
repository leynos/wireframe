//! Client example that logs in against the `echo` server.
//!
//! Start `examples/echo.rs` first, then run this example. The client sends a
//! typed login request inside an [`wireframe::app::Envelope`], receives the
//! echoed envelope, and decodes the acknowledgement payload.

use std::net::SocketAddr;

use tracing::{error, info};
use wireframe::{
    app::Envelope,
    client::WireframeClient,
    correlation::CorrelatableFrame,
    message::Message,
};

#[path = "support/echo_login_contract.rs"]
mod echo_login_contract;

use echo_login_contract::{LOGIN_ROUTE_ID, LoginAck, LoginRequest};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let addr: SocketAddr = "127.0.0.1:7878".parse()?;
    let mut client = WireframeClient::builder().connect(addr).await?;

    let login = LoginRequest {
        username: "guest".to_string(),
    };
    let payload = login.to_bytes()?;
    let request = Envelope::new(LOGIN_ROUTE_ID, None, payload);
    let response: Envelope = client.call_correlated(request).await?;

    let (ack, _) = LoginAck::from_bytes(response.payload_bytes())?;
    info!(
        username = %ack.username,
        correlation_id = ?response.correlation_id(),
        "decoded login acknowledgement",
    );

    if ack.username != login.username {
        error!(
            sent = %login.username,
            received = %ack.username,
            "login acknowledgement mismatch",
        );
        let error = std::io::Error::other(format!(
            "login acknowledgement mismatch: sent '{}', received '{}'",
            login.username, ack.username
        ));
        return Err(error.into());
    }

    info!("client echo-login example completed successfully");
    Ok(())
}
