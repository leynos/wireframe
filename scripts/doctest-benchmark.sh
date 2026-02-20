#!/usr/bin/env bash
set -euo pipefail

MIN_RUNNABLE="${DOCTEST_MIN_RUNNABLE_PCT:-70}"
MAX_NO_RUN="${DOCTEST_MAX_NO_RUN_PCT:-30}"

if [[ "$#" -gt 0 ]]; then
  search_roots=("$@")
elif [[ -n "${DOCTEST_BENCHMARK_ROOTS:-}" ]]; then
  # shellcheck disable=SC2206
  search_roots=(${DOCTEST_BENCHMARK_ROOTS})
else
  search_roots=(src)
fi

collect_rust_files() {
  if command -v rg >/dev/null 2>&1; then
    rg --files "${search_roots[@]}" -g '*.rs' | sort
    return
  fi
  find "${search_roots[@]}" -type f -name '*.rs' | sort
}

mapfile -t rust_files < <(collect_rust_files)
if [[ "${#rust_files[@]}" -eq 0 ]]; then
  echo "ERROR: no Rust source files found in roots: ${search_roots[*]}"
  exit 1
fi

read -r runnable no_run ignored non_rust < <(
  awk '
    /^[[:space:]]*\/\/[\/!][[:space:]]*```/ {
      line = $0
      sub(/^[[:space:]]*\/\/[\/!][[:space:]]*```/, "", line)
      info = tolower(line)
      gsub(/^[[:space:]]+|[[:space:]]+$/, "", info)

      if (in_block == 0) {
        if (info ~ /(^|,)no_run(,|$)/) {
          no_run += 1
        } else if (info ~ /(^|,)(ignore|compile_fail)(,|$)/) {
          ignored += 1
        } else if (info == "" || info ~ /^rust($|,)/) {
          runnable += 1
        } else {
          non_rust += 1
        }
        in_block = 1
      } else {
        in_block = 0
      }
    }

    END {
      printf "%d %d %d %d\n", runnable, no_run, ignored, non_rust
    }
  ' "${rust_files[@]}"
)

eligible=$((runnable + no_run))
if [[ "$eligible" -eq 0 ]]; then
  echo "ERROR: no runnable/no_run doctest blocks found"
  exit 1
fi

runnable_lhs=$((runnable * 100))
no_run_lhs=$((no_run * 100))
runnable_pct="$(awk -v n="$runnable" -v d="$eligible" 'BEGIN { printf "%.2f", (n * 100) / d }')"
no_run_pct="$(awk -v n="$no_run" -v d="$eligible" 'BEGIN { printf "%.2f", (n * 100) / d }')"

printf 'Doctest benchmark\n'
printf '  runnable: %d\n' "$runnable"
printf '  no_run: %d\n' "$no_run"
printf '  ignored-or-compile_fail: %d\n' "$ignored"
printf '  non-rust-or-text: %d\n' "$non_rust"
printf '  runnable_ratio: %s%%\n' "$runnable_pct"
printf '  no_run_ratio: %s%%\n' "$no_run_pct"

if (( runnable_lhs < MIN_RUNNABLE * eligible )); then
  echo "ERROR: runnable ratio ${runnable_pct}% is below ${MIN_RUNNABLE}%"
  exit 1
fi

if (( no_run_lhs > MAX_NO_RUN * eligible )); then
  echo "ERROR: no_run ratio ${no_run_pct}% exceeds ${MAX_NO_RUN}%"
  exit 1
fi
