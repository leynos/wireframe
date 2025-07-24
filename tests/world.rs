use std::net::SocketAddr;

use cucumber::World;
use tokio::sync::oneshot::Sender;

#[derive(Debug, Default, World)]
pub struct PanicWorld {
    pub addr: Option<SocketAddr>,
    pub attempts: usize,
    pub shutdown: Option<Sender<()>>,
}
