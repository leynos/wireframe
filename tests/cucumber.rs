mod steps;
mod world;

use cucumber::World;
use world::PanicWorld;

#[tokio::main]
async fn main() { PanicWorld::run("tests/features").await; }
