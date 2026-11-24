//! The message assembler accessors hand out a borrow, not a cloneable handle.
//!
//! Issue #598 asked what an application should hand back once it owns its
//! assembler outright. The answer in the 0.4.0 release is a borrowed
//! `Option<&dyn MessageAssembler>`: callers invoke the assembler through the
//! reference and cannot clone it out of the application. This fixture pins the
//! builder and prepared accessor types.
use std::io;

use wireframe::{
    app::{Envelope, PreparedApp, WireframeApp},
    message_assembler::{MessageAssembler, ParsedFrameHeader},
    serializer::BincodeSerializer,
};

/// Assembler that never accepts a frame header.
struct Demo;

impl MessageAssembler for Demo {
    fn parse_frame_header(&self, _payload: &[u8]) -> io::Result<ParsedFrameHeader> {
        Err(io::Error::other("this fixture never assembles"))
    }
}

/// Confirm the builder accessor returns a plain borrow.
fn borrow_from_builder(app: &WireframeApp<BincodeSerializer, (), Envelope>) {
    let assembler: Option<&dyn MessageAssembler> = app.message_assembler();
    assert!(assembler.is_some(), "the builder should expose its assembler");
}

/// Confirm the prepared accessor returns a plain borrow.
async fn borrow_from_prepared(
    app: WireframeApp<BincodeSerializer, (), Envelope>,
) -> Result<(), Box<dyn std::error::Error>> {
    let prepared: PreparedApp<BincodeSerializer, (), Envelope> = app.prepare().await?;
    let assembler: Option<&dyn MessageAssembler> = prepared.message_assembler();
    assert!(
        assembler.is_some(),
        "the prepared application should expose its assembler"
    );
    Ok(())
}

#[tokio::main]
async fn main() {
    let app =
        WireframeApp::<BincodeSerializer, (), Envelope>::default().with_message_assembler(Demo);
    borrow_from_builder(&app);
    borrow_from_prepared(app)
        .await
        .expect("the prepared accessor should be usable");
}