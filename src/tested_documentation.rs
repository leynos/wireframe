//! Published user-facing guides included in the crate's rustdoc test suite.

#[doc = include_str!("../docs/api.md")]
pub mod api {}

#[doc = include_str!("../docs/developers-guide.md")]
pub mod developers_guide {}

#[doc = include_str!("../docs/users-guide.md")]
pub mod users_guide {}

#[doc = include_str!("../docs/v0-1-0-to-v0-2-0-migration-guide.md")]
pub mod migration_v0_1_to_v0_2 {}

#[doc = include_str!("../docs/v0-2-0-to-v0-3-0-migration-guide.md")]
pub mod migration_v0_2_to_v0_3 {}

#[doc = include_str!("../docs/v0-3-0-to-v0-4-0-migration-guide.md")]
pub mod migration_v0_3_to_v0_4 {}

#[doc = include_str!("../docs/rstest-bdd-v0-6-0-migration-guide.md")]
pub mod rstest_bdd_migration {}

#[doc = include_str!("../docs/hardening-wireframe-a-guide-to-production-resilience.md")]
pub mod resilience_guide {}

#[doc = include_str!("../docs/wireframe-testing-crate.md")]
pub mod testing_crate_guide {}

#[doc = include_str!("../docs/rstest-bdd-users-guide.md")]
pub mod rstest_bdd_users_guide {}

#[doc = include_str!("../docs/wireframe-rstest-bdd-users-guide.md")]
pub mod wireframe_rstest_bdd_users_guide {}

#[doc = include_str!("../docs/whitaker-users-guide.md")]
pub mod whitaker_users_guide {}

#[doc = include_str!("../docs/server/configuration.md")]
pub mod server_configuration {}

#[doc = include_str!("../docs/mocking-network-outages-in-rust.md")]
pub mod mocking_network_outages {}

#[doc = include_str!("../docs/rust-testing-with-rstest-fixtures.md")]
pub mod rstest_fixtures_guide {}

#[doc = include_str!("../docs/behavioural-testing-in-rust-with-cucumber.md")]
pub mod cucumber_migration_guide {}

#[doc = include_str!("../docs/rust-doctest-dry-guide.md")]
pub mod rustdoc_examples_guide {}

#[doc = include_str!("../examples/async_stream.md")]
pub mod async_stream_example {}

#[doc = include_str!("../examples/multi_packet.md")]
pub mod multi_packet_example {}

#[doc = include_str!("../examples/ping_pong.md")]
pub mod ping_pong_example {}
