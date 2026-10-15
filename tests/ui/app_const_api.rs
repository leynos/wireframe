//! Verifies public application configuration and packet constructors in const contexts.

use wireframe::{
    app::{Envelope, MemoryBudgets, PacketParts, WireframeApp},
    fragment::FragmentationConfig,
};

const ENVELOPE: Envelope = Envelope::new(7, Some(3), Vec::new());
const PACKET_PARTS: PacketParts = PacketParts::new(7, Some(3), Vec::new());

const fn configure_app(
    app: WireframeApp,
    fragmentation: Option<FragmentationConfig>,
    budgets: MemoryBudgets,
) -> WireframeApp {
    let _ = app.codec();
    app.fragmentation(fragmentation).memory_budgets(budgets)
}

fn main() {
    let _ = (ENVELOPE, PACKET_PARTS, configure_app);
}
