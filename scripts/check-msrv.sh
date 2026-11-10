#!/usr/bin/env bash
# Prove that the Rust version the manifest declares can still compile Wireframe.
#
# The version comes from `rust-version` in Cargo.toml, so there is no second
# copy to drift. Two checks answer two different questions:
#
#   1. The workspace, from the committed lock file: does our own code, including
#      its tests, examples and benches, compile on the minimum?
#   2. A throwaway consumer, with freshly resolved dependencies: can someone who
#      adds Wireframe today compile it? A lock file cannot show this, because
#      dependency releases reach users without passing through it.
#
# Both are `cargo check`, never a build or a test run: the estate's cost rule
# for an MSRV job (concordat RT-026) is `cargo check --all-targets` on the
# minimum toolchain, because the pinned nightly already builds and tests
# everything.

set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
msrv=$(sed -n 's/^rust-version *= *"\(.*\)"$/\1/p' "$repo_root/Cargo.toml")
if [[ -z $msrv ]]; then
    echo "msrv: Cargo.toml declares no rust-version" >&2
    exit 1
fi

# A separate target directory, so these artefacts cannot displace the pinned
# nightly's. Exported so the consumer shares it and reuses unchanged dependencies.
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$repo_root/target/msrv}"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

echo "==> Installing Rust $msrv"
rustup toolchain install "$msrv" --profile minimal --no-self-update

echo "==> Checking the workspace on Rust $msrv from the lock file"
# .cargo/config.toml passes `-Zthreads`, which stable compilers reject. Assigning
# RUSTFLAGS displaces every configured source, and a blank value clears them.
(
    cd "$repo_root"
    RUSTFLAGS=" " cargo "+$msrv" check --locked --workspace --all-targets --all-features
)

echo "==> Checking a consumer on Rust $msrv with fresh dependencies"
# Outside the repository, so this sees none of its Cargo configuration, just as
# a real consumer does not. The stated rust-version makes the resolver choose
# dependency versions the way it would for a consumer holding the same floor.
mkdir "$work/src"
cat >"$work/Cargo.toml" <<MANIFEST
[package]
name = "msrv-consumer"
version = "0.0.0"
edition = "2024"
rust-version = "$msrv"
publish = false

[dependencies]
wireframe = { path = "$repo_root", features = ["pool", "serializer-serde"] }
wireframe_testing = { path = "$repo_root/wireframe_testing" }

[workspace]
MANIFEST
cat >"$work/src/lib.rs" <<'SOURCE'
pub use wireframe;
pub use wireframe_testing;
SOURCE
(
    cd "$work"
    cargo "+$msrv" check
)

echo "==> Rust $msrv compiles Wireframe"
