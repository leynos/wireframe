//! Verifies `Result` uses `WireframeError<NoProtocolError>` by default.

use wireframe::{NoProtocolError, Result, WireframeError};

fn accepts_error(_: &dyn std::error::Error) {}

fn accepts_result(_: Result<()>) {}

fn main() {
    let default_error: WireframeError = WireframeError::DuplicateRoute(1);
    accepts_error(&default_error);

    let explicit_default: WireframeError<NoProtocolError> = WireframeError::DuplicateRoute(2);
    accepts_error(&explicit_default);

    accepts_result(Err(WireframeError::DuplicateRoute(3)));
}
