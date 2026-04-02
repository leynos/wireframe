//! Client example that logs in against the `echo` server.
//!
//! Start `examples/echo.rs` first, then run this example. The client sends a
//! typed login request inside an [`wireframe::app::Envelope`], receives the
//! echoed envelope, and decodes the acknowledgement payload.

use std::net::SocketAddr;

use tokio::net::TcpStream;
use tracing::{error, info};
use wireframe::{
    app::Envelope,
    client::WireframeClient,
    correlation::CorrelatableFrame,
    message::Message,
    rewind_stream::RewindStream,
    serializer::BincodeSerializer,
};

#[path = "support/echo_login_contract.rs"]
mod echo_login_contract;

use echo_login_contract::{LOGIN_ROUTE_ID, LoginAck, LoginRequest};

type ExampleResult<T> = Result<T, Box<dyn std::error::Error>>;
type Client = WireframeClient<BincodeSerializer, RewindStream<TcpStream>, ()>;

fn init_tracing() { let _ = tracing_subscriber::fmt::try_init(); }

fn validate_acknowledgement(login: &LoginRequest, ack: &LoginAck) -> ExampleResult<()> {
    if ack.username == login.username {
        return Ok(());
    }

    error!(
        sent = %login.username,
        received = %ack.username,
        "login acknowledgement mismatch",
    );
    let error = std::io::Error::other(format!(
        "login acknowledgement mismatch: sent '{}', received '{}'",
        login.username, ack.username
    ));
    Err(error.into())
}

async fn request_acknowledgement(
    client: &mut Client,
    login: &LoginRequest,
) -> ExampleResult<(Envelope, LoginAck)> {
    let payload = login.to_bytes()?;
    let request = Envelope::new(LOGIN_ROUTE_ID, None, payload);
    let response: Envelope = client.call_correlated(request).await?;
    let (ack, _) = LoginAck::from_bytes(response.payload_bytes())?;
    Ok((response, ack))
}

#[expect(
    clippy::cognitive_complexity,
    reason = "The example keeps a linear connect-send-decode-validate flow for readability"
)]
async fn run() -> Result<(), Box<dyn std::error::Error>> {
    init_tracing();

    let addr: SocketAddr = "127.0.0.1:7878".parse()?;
    let mut client: Client = WireframeClient::builder().connect(addr).await?;

    let login = LoginRequest {
        username: "guest".to_string(),
    };

    let (response, ack) = request_acknowledgement(&mut client, &login).await?;

    info!(
        username = %ack.username,
        correlation_id = ?response.correlation_id(),
        "decoded login acknowledgement",
    );

    validate_acknowledgement(&login, &ack)?;
    info!("client echo-login example completed successfully");
    Ok(())
}

fn main() -> ExampleResult<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(run())
}
