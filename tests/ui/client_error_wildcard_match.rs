//! A downstream match can handle known errors and reserve a fallback arm.

use wireframe::client::ClientError;

fn saturation_limit(error: ClientError) -> Option<usize> {
    match error {
        ClientError::PoolSaturated { limit } => Some(limit),
        _ => None,
    }
}

fn main() {
    let _ = saturation_limit as fn(ClientError) -> Option<usize>;
}
