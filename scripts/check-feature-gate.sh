#!/usr/bin/env bash
# Verify that the client pool's `test-support` accessors really are gated.
#
# Trybuild cannot prove this: it forwards the harness's own feature list to its
# fixture, and the root manifest's `wireframe_testing` development dependency
# enables `wireframe/test-support` for every fixture in this repository. This
# script therefore builds a standalone consumer (`tests/feature_gate`) that
# depends on `wireframe` alone and drives it through three configurations:
#
#   1. accessors disabled  -> must FAIL, naming `pool_size` and `slot_id`
#   2. accessors enabled   -> must PASS
#   3. public API, disabled -> must PASS
#
# Run 3 is the control. Without it, a consumer so broken that nothing compiles
# would make run 1's expected failure meaningless.
#
# Each run is a separate `cargo check`, so this is not cheap; it is a
# compile-time correctness gate, not a fast unit test.

set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
consumer="$repo_root/tests/feature_gate"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

fail() {
    echo "feature-gate: $*" >&2
    exit 1
}

run_check() {
    local label=$1
    local log=$2
    shift 2

    echo "==> $label"
    if cargo check --quiet --manifest-path "$consumer/Cargo.toml" "$@" >"$log" 2>&1; then
        echo "    ok"
        return 0
    fi

    echo "    failed (log: $log)" >&2
    return 1
}

# The consumer is its own workspace, so `cargo fmt --all` at the repository root
# does not reach it.
echo "==> consumer formatting"
cargo fmt --manifest-path "$consumer/Cargo.toml" --all -- --check ||
    fail "the feature-gate consumer is not formatted"

echo "==> consumer lint"
# `--all-features` compiles the probe too, so a probe that no longer fails for
# the right reason still has to be lint-clean.
cargo clippy --quiet --manifest-path "$consumer/Cargo.toml" \
    --all-targets --all-features -- -D warnings >"$work/clippy.log" 2>&1 ||
    {
        cat "$work/clippy.log" >&2
        fail "the feature-gate consumer has Clippy findings"
    }

# 1. Disabled: the probe must fail to compile, and only because of the gate.
if run_check "accessors disabled (expected to fail)" "$work/disabled.log"; then
    fail "the probe compiled without test-support; pool_size/slot_id are not gated"
fi

for method in pool_size slot_id; do
    if ! grep -q "no method named \`$method\`" "$work/disabled.log"; then
        cat "$work/disabled.log" >&2
        fail "the disabled build failed, but not on \`$method\`; the probe is not evidence"
    fi
done
echo "    both accessors reported missing, as intended"

# 2. Enabled: the same calls must compile.
run_check "accessors enabled (expected to pass)" "$work/enabled.log" \
    --lib --features test-support ||
    fail "the accessors did not compile with test-support enabled"

# 3. Control: the stable public surface must compile with the feature off.
run_check "public API without test-support (expected to pass)" "$work/control.log" \
    --lib ||
    fail "the public pool API does not compile without test-support"

echo "feature-gate: ok"
