//! Exhaustive downstream matches are rejected for non-exhaustive client errors.

use wireframe::client::ClientError;

fn match_known_variants(error: ClientError) {
    match error {
        ClientError::Wireframe(_) => {}
        ClientError::Serialize(_) => {}
        ClientError::PreambleEncode(_) => {}
        ClientError::PreambleWrite(_) => {}
        ClientError::PreambleRead(_) => {}
        ClientError::PreambleTimeout => {}
        ClientError::CorrelationMismatch { .. } => {}
        ClientError::StreamCorrelationMismatch { .. } => {}
        ClientError::PoolSaturated { .. } => {}
    }
}

fn main() {
    let _ = match_known_variants as fn(ClientError);
}
