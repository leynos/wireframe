#!/usr/bin/env bash
# Prove that the workspace's `unsafe_code` prohibition actually rejects unsafe Rust.
#
# `tests/workspace_unsafe_boundary.rs` can show that every safe Cargo target root
# carries `#![forbid(unsafe_code)]`. It cannot show that the attribute does
# anything: renaming a lint, or a toolchain that stops applying crate-level
# `forbid` to macro output, would leave that textual contract satisfied while the
# prohibition evaporated. This script supplies the other half by compiling code
# that must fail:
#
#   * one control crate that carries the prohibition and only safe code, which
#     must compile — without it, "the probe failed" would also be satisfied by a
#     toolchain that rejects everything;
#   * one probe per unsafe construct the prohibition is expected to cover, each
#     of which must be rejected by the `unsafe_code` lint itself;
#   * one probe showing that `#[expect(unsafe_code)]` is *overruled* by
#     `forbid`, which is the finding that forced the audited boundary crate to
#     carry a scoped `#[expect]` under a workspace-wide `deny` floor instead of
#     a `forbid`;
#   * one control showing that the same expectation *is* accepted under `deny`,
#     which is how `crates/wireframe-bench-alloc` is written.
#
# Diagnostics are read from Cargo's JSON message stream rather than from the
# rendered text: the JSON carries the lint name, so a probe that fails for an
# unrelated reason (a typo, a missing dependency, a deny-by-default warning)
# cannot be mistaken for a passing gate.
#
# The probes are built with the toolchain pinned in `rust-toolchain.toml`, in a
# temporary package outside the repository, so they neither join the workspace
# nor disturb the contract test's target-root inventory.

set -euo pipefail

repo_root=$(git rev-parse --show-toplevel)
toolchain=$(sed -n 's/^channel = "\(.*\)"$/\1/p' "$repo_root/rust-toolchain.toml")

if [[ -z "$toolchain" ]]; then
	printf 'check-unsafe-boundary: cannot read the channel pin from rust-toolchain.toml\n' >&2
	exit 1
fi

work=$(mktemp -d "${TMPDIR:-/tmp}/unsafe-boundary.XXXXXX")
trap 'rm -rf "$work"' EXIT

probe_dir="$work/probe"
mkdir -p "$probe_dir/src"

# A standalone package, so Cargo never associates the probe with the repository's
# workspace manifest.
cat >"$probe_dir/Cargo.toml" <<'TOML'
[package]
name = "unsafe-boundary-probe"
version = "0.0.0"
edition = "2024"
publish = false

[workspace]
TOML

failures=0

pass() { printf 'ok   %s\n' "$1"; }
fail() {
	printf 'FAIL %s\n' "$1"
	failures=$((failures + 1))
}

# Compile one probe source and leave the diagnostics in `$work`.
#
# Returns Cargo's status; callers decide whether that status is the expected one.
compile_probe() {
	printf '%s\n' "$1" >"$probe_dir/src/lib.rs"
	(
		cd "$work"
		cargo "+$toolchain" check \
			--manifest-path "$probe_dir/Cargo.toml" \
			--message-format=json
	) >"$work/messages.json" 2>"$work/stderr.log"
}

# Every diagnostic code Cargo emitted, one per line, without duplicates.
diagnostic_codes() {
	jq -r 'select(.reason == "compiler-message") | .message.code.code // empty' \
		"$work/messages.json" | sort -u
}

expect_acceptance() {
	local name=$1 source=$2
	if compile_probe "$source"; then
		pass "$name"
	else
		fail "$name: the probe was rejected, but it should compile (codes: $(diagnostic_codes | paste -sd, -))"
		sed -n '1,20p' "$work/stderr.log" >&2
	fi
}

expect_lint_rejection() {
	local name=$1 source=$2 codes
	if compile_probe "$source"; then
		fail "$name: the probe compiled, so the prohibition did not reject unsafe code"
		return
	fi
	codes=$(diagnostic_codes)
	if grep -qx unsafe_code <<<"$codes"; then
		pass "$name"
	else
		fail "$name: rejected, but not by the unsafe_code lint (codes: $(paste -sd, - <<<"$codes"))"
		sed -n '1,20p' "$work/stderr.log" >&2
	fi
}

expect_forbid_overrules_expectation() {
	local name=$1 source=$2 codes
	if compile_probe "$source"; then
		fail "$name: the crate compiled, so an expectation escaped a crate-level forbid"
		return
	fi
	codes=$(diagnostic_codes)
	if grep -qx E0453 <<<"$codes"; then
		pass "$name"
	else
		fail "$name: rejected, but not as an expectation incompatible with a previous forbid (codes: $(paste -sd, - <<<"$codes"))"
		sed -n '1,20p' "$work/stderr.log" >&2
	fi
}

expect_acceptance "control: safe code compiles under forbid" \
	'#![forbid(unsafe_code)]

pub fn safe_probe() -> u32 { 42 }'

expect_lint_rejection "unsafe block is rejected" \
	'#![forbid(unsafe_code)]

pub fn probe() -> u32 {
    let value = 1_u32;
    unsafe { std::ptr::read(&value) }
}'

expect_lint_rejection "unsafe function is rejected" \
	'#![forbid(unsafe_code)]

pub unsafe fn probe() -> u32 { 0 }'

expect_lint_rejection "unsafe trait is rejected" \
	'#![forbid(unsafe_code)]

pub unsafe trait Probe {}'

expect_lint_rejection "unsafe implementation is rejected" \
	'#![forbid(unsafe_code)]

pub struct Probe;
unsafe impl Send for Probe {}'

expect_lint_rejection "unsafe extern block is rejected" \
	'#![forbid(unsafe_code)]

unsafe extern "C" {
    fn probe();
}'

expect_forbid_overrules_expectation "forbid overrules a scoped expectation" \
	'#![forbid(unsafe_code)]

pub struct Probe;

#[expect(unsafe_code, reason = "probing whether forbid overrules an expectation")]
unsafe impl Send for Probe {}'

expect_acceptance "control: deny accepts a scoped expectation" \
	'#![deny(unsafe_code)]

pub struct Probe;

#[expect(unsafe_code, reason = "the workspace tests unsafe_code as a deny floor")]
unsafe impl Send for Probe {}'

if ((failures > 0)); then
	printf '\ncheck-unsafe-boundary: %d probe(s) failed\n' "$failures" >&2
	exit 1
fi

printf '\ncheck-unsafe-boundary: all probes behaved as expected\n'
