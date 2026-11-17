//! Verifies the removed extractor and application-data APIs stay removed.

use wireframe::{
    app::WireframeApp,
    extractor::SharedState,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _app = WireframeApp::new()?.app_data(5u32);
    Ok(())
}