#!/bin/sh
# Placeholder runner for formal-verification targets until their owning roadmap
# items add real harnesses or proofs. Strict mode turns a skip into a CI guard.
set -eu

target="${1:?target name required}"
shift
message="$*"

printf 'FORMAL-SKIP: %s not yet implemented — %s\n' "$target" "$message" >&2

if [ -n "${FORMAL_STRICT:-}" ]; then
	exit 1
fi
