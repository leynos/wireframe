#!/usr/bin/env bash
set -euo pipefail

MIN_RUNNABLE=70
MAX_NO_RUN=30

mapfile -t rust_files < <(rg --files src -g '*.rs' | sort)

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

runnable_pct=$((runnable * 100 / eligible))
no_run_pct=$((no_run * 100 / eligible))

printf 'Doctest benchmark\n'
printf '  runnable: %d\n' "$runnable"
printf '  no_run: %d\n' "$no_run"
printf '  ignored-or-compile_fail: %d\n' "$ignored"
printf '  non-rust-or-text: %d\n' "$non_rust"
printf '  runnable_ratio: %d%%\n' "$runnable_pct"
printf '  no_run_ratio: %d%%\n' "$no_run_pct"

if (( runnable_pct < MIN_RUNNABLE )); then
  echo "ERROR: runnable ratio ${runnable_pct}% is below ${MIN_RUNNABLE}%"
  exit 1
fi

if (( no_run_pct > MAX_NO_RUN )); then
  echo "ERROR: no_run ratio ${no_run_pct}% exceeds ${MAX_NO_RUN}%"
  exit 1
fi
