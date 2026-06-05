//! Verifies the `WireframeError` default is `NoProtocolError`, not `()`.
//! This blocks regressions to `E = ()`; see #513 for the API rationale.

use wireframe::{NoProtocolError, Result, WireframeError};

fn main() {
    let _: WireframeError = WireframeError::Protocol(());
    let _: WireframeError<NoProtocolError> = WireframeError::Protocol(());
    let _: Result<()> = Err(WireframeError::Protocol(()));
}
