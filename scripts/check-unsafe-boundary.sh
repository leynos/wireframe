#!/usr/bin/env bash
# Prove that the workspace's `unsafe_code` prohibition actually rejects unsafe Rust.
#
# `tests/workspace_unsafe_boundary.rs` can show that every Cargo target root
# carries `#![forbid(unsafe_code)]` and that no package source spells an unsafe
# construct. It cannot show that the attribute does anything: renaming a lint, or
# a toolchain that stops applying crate-level `forbid` to macro output, would
# leave that textual contract satisfied while the prohibition evaporated. This
# script supplies the other half by compiling code that must fail:
#
#   * a control crate that carries the prohibition and only safe code, which must
#     compile — without it, "the probe failed" would also be satisfied by a
#     toolchain that rejects everything;
#   * one probe per unsafe construct the prohibition is expected to cover, each
#     of which must be rejected by the `unsafe_code` lint itself;
#   * one probe showing that `#[expect(unsafe_code)]` is *overruled* by
#     `forbid`, so a scoped expectation cannot reopen the prohibition;
#   * a second package whose prohibition comes from its manifest's
#     `[lints.rust]` table rather than from an attribute, with a safe control and
#     an unsafe probe, because the workspace table is the other half of the
#     contract and it reaches packages that never write the attribute.
#
# Diagnostics are read from Cargo's JSON message stream rather than from the
# rendered text: the JSON carries the lint name, so a probe that fails for an
# unrelated reason (a typo, a missing dependency, a deny-by-default warning)
# cannot be mistaken for a passing gate.
#
# The probes are built with the toolchain pinned in `rust-toolchain.toml`, in
# temporary packages outside the repository, so they neither join the workspace
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

# Two standalone packages, so Cargo never associates a probe with the
# repository's workspace manifest. The first states the prohibition in its
# source; the second states it in its manifest.
attribute_probe="$work/attribute-probe"
manifest_probe="$work/manifest-probe"
mkdir -p "$attribute_probe/src" "$manifest_probe/src"
attribute_manifest="$attribute_probe/Cargo.toml"
manifest_manifest="$manifest_probe/Cargo.toml"

cat >"$attribute_manifest" <<'TOML'
[package]
name = "unsafe-boundary-attribute-probe"
version = "0.0.0"
edition = "2024"
publish = false

[workspace]
TOML

cat >"$manifest_manifest" <<'TOML'
[package]
name = "unsafe-boundary-manifest-probe"
version = "0.0.0"
edition = "2024"
publish = false

[workspace]

[lints.rust]
unsafe_code = "forbid"
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
	local manifest=$1 source=$2
	printf '%s\n' "$source" >"$(dirname "$manifest")/src/lib.rs"
	(
		cd "$work"
		cargo "+$toolchain" check \
			--manifest-path "$manifest" \
			--message-format=json
	) >"$work/messages.json" 2>"$work/stderr.log"
}

# Every diagnostic code Cargo emitted, one per line, without duplicates.
diagnostic_codes() {
	jq -r 'select(.reason == "compiler-message") | .message.code.code // empty' \
		"$work/messages.json" | sort -u
}

expect_acceptance() {
	local name=$1 manifest=$2 source=$3
	if compile_probe "$manifest" "$source"; then
		pass "$name"
	else
		fail "$name: the probe was rejected, but it should compile (codes: $(diagnostic_codes | paste -sd, -))"
		sed -n '1,20p' "$work/stderr.log" >&2
	fi
}

expect_lint_rejection() {
	local name=$1 manifest=$2 source=$3 codes
	if compile_probe "$manifest" "$source"; then
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
	local name=$1 manifest=$2 source=$3 codes
	if compile_probe "$manifest" "$source"; then
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

expect_acceptance "control: safe code compiles under a crate-level forbid" \
	"$attribute_manifest" \
	'#![forbid(unsafe_code)]

pub fn safe_probe() -> u32 { 42 }'

expect_lint_rejection "unsafe block is rejected" \
	"$attribute_manifest" \
	'#![forbid(unsafe_code)]

pub fn probe() -> u32 {
    let value = 1_u32;
    unsafe { std::ptr::read(&value) }
}'

expect_lint_rejection "unsafe function is rejected" \
	"$attribute_manifest" \
	'#![forbid(unsafe_code)]

pub unsafe fn probe() -> u32 { 0 }'

expect_lint_rejection "unsafe trait is rejected" \
	"$attribute_manifest" \
	'#![forbid(unsafe_code)]

pub unsafe trait Probe {}'

expect_lint_rejection "unsafe implementation is rejected" \
	"$attribute_manifest" \
	'#![forbid(unsafe_code)]

pub struct Probe;
unsafe impl Send for Probe {}'

expect_lint_rejection "unsafe extern block is rejected" \
	"$attribute_manifest" \
	'#![forbid(unsafe_code)]

unsafe extern "C" {
    fn probe();
}'

expect_forbid_overrules_expectation "forbid overrules a scoped expectation" \
	"$attribute_manifest" \
	'#![forbid(unsafe_code)]

pub struct Probe;

#[expect(unsafe_code, reason = "probing whether forbid overrules an expectation")]
unsafe impl Send for Probe {}'

expect_acceptance "control: safe code compiles under a manifest-level forbid" \
	"$manifest_manifest" \
	'pub fn safe_probe() -> u32 { 42 }'

expect_lint_rejection "manifest-level forbid rejects an unsafe block" \
	"$manifest_manifest" \
	'pub fn probe() -> u32 {
    let value = 1_u32;
    unsafe { std::ptr::read(&value) }
}'

if ((failures > 0)); then
	printf '\ncheck-unsafe-boundary: %d probe(s) failed\n' "$failures" >&2
	exit 1
fi

printf '\ncheck-unsafe-boundary: all probes behaved as expected\n'
