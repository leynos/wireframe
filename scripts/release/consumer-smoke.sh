#!/usr/bin/env bash
# Build a throwaway consumer of a published release, from crates.io alone.
#
# The consumer lives outside the repository, so it sees none of its Cargo
# configuration and no path dependency, exactly as a reader of the docs would.

set -euo pipefail

if [[ $# -ne 1 ]]; then
    echo "usage: consumer-smoke.sh <version>" >&2
    exit 2
fi
version=$1
repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)

# The minimum Rust version users are promised, rather than the pinned nightly.
msrv=$(sed -n 's/^rust-version *= *"\(.*\)"$/\1/p' "$repo_root/Cargo.toml")
if [[ -z $msrv ]]; then
    echo "consumer smoke: Cargo.toml declares no rust-version" >&2
    exit 1
fi
rustup toolchain install "$msrv" --profile minimal --no-self-update

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
mkdir "$work/src"
cat >"$work/Cargo.toml" <<MANIFEST
[package]
name = "release-consumer"
version = "0.0.0"
edition = "2024"
rust-version = "$msrv"
publish = false

[dependencies]
wireframe = "=$version"
wireframe_testing = "=$version"

[workspace]
MANIFEST
cat >"$work/src/lib.rs" <<'SOURCE'
//! Throwaway consumer of a published release.

pub use wireframe;
pub use wireframe_testing;
SOURCE
(
    cd "$work"
    cargo "+$msrv" check
)
echo "consumer smoke: wireframe $version and wireframe_testing $version build"
