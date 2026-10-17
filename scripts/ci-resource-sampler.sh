#!/usr/bin/env bash
# Sample peak memory use, minimum free disk and busy cores while a CI job runs.
#
# MEASUREMENT BRANCHES ONLY (jm5-placement-trials, DF12-91): this CPU-reading
# variant of axinite's sampler is never merged. Busy cores come from the
# aggregate `cpu` line of /proc/stat:
#   busy  = user + nice + system + irq + softirq   (guest is already in user)
#   total = busy + idle + iowait + steal
#   busy_cores = delta(busy) / delta(total) * nproc
# Steal is excluded from busy and reported separately.
#
# Right-sizing a runner needs two numbers the job does not otherwise report:
# how much memory it actually used at its peak, and how close it came to
# filling the volume. Both belong to the shape, not to the workload: Cargo
# scales parallelism with the processor count, so a peak measured on one shape
# does not transfer to another. Sample on the shape you intend to use.
#
# Usage:
#   ci-resource-sampler.sh start    # begin sampling in the background
#   ci-resource-sampler.sh report   # stop, print the peaks, write the summary
#
# The sampler wakes every SAMPLER_INTERVAL_SECONDS (default 10) and shells out,
# which is negligible for a compile job and unacceptable for one whose output
# is a timing. Do not run it on a benchmark job.

set -euo pipefail

STATE_DIR="${RUNNER_TEMP:-/tmp}/ci-resource-sampler"
PEAK_FILE="${STATE_DIR}/peak-memory-kib"
FREE_FILE="${STATE_DIR}/min-free-kib"
PID_FILE="${STATE_DIR}/sampler.pid"
CPU_FILE="${STATE_DIR}/cpu-samples"
CPU_LAST_FILE="${STATE_DIR}/cpu-last"
START_FILE="${STATE_DIR}/start-epoch"
INTERVAL="${SAMPLER_INTERVAL_SECONDS:-10}"

# Memory in use is total minus available, not total minus free: page cache is
# reclaimable, and counting it as used would report every job as near its
# limit.
used_memory_kib() {
  awk '/^MemTotal:/ {t = $2} /^MemAvailable:/ {a = $2} END {print t - a}' \
    /proc/meminfo
}

free_disk_kib() {
  df -Pk . | awk 'NR == 2 {print $4}'
}

# Write through a temporary file and rename, so a reader sees either the old
# value or the new one and never a truncated file. This is not sufficient on
# its own, because the update is a read-modify-write, but it removes the
# empty-file window that `set -u` would turn into a failed report.
write_atomic() {
  local path="$1" value="$2"
  printf '%s\n' "${value}" > "${path}.tmp"
  mv -f "${path}.tmp" "${path}"
}

# Print "busy total steal" cumulative jiffies from the aggregate cpu line.
cpu_counters() {
  awk '/^cpu / {
    busy = $2 + $3 + $4 + $7 + $8
    total = busy + $5 + $6 + $9
    print busy, total, $9
    exit
  }' /proc/stat
}

# Append one "dbusy dtotal dsteal" line per interval, relative to the previous
# reading (kept on disk so `report` can take the final interval too).
sample_cpu() {
  local now busy total steal pbusy ptotal psteal
  now="$(cpu_counters)"
  if [ -s "${CPU_LAST_FILE}" ]; then
    read -r busy total steal <<< "${now}"
    read -r pbusy ptotal psteal < "${CPU_LAST_FILE}"
    if [ "$((total - ptotal))" -gt 0 ]; then
      printf '%s %s %s\n' "$((busy - pbusy))" "$((total - ptotal))" \
        "$((steal - psteal))" >> "${CPU_FILE}"
    fi
  fi
  write_atomic "${CPU_LAST_FILE}" "${now}"
}

sample_once() {
  local used free previous
  sample_cpu
  used="$(used_memory_kib)"
  free="$(free_disk_kib)"
  previous="$(cat "${PEAK_FILE}")"
  if [ "${used}" -gt "${previous}" ]; then
    write_atomic "${PEAK_FILE}" "${used}"
  fi
  previous="$(cat "${FREE_FILE}")"
  if [ "${free}" -lt "${previous}" ]; then
    write_atomic "${FREE_FILE}" "${free}"
  fi
}

start_sampler() {
  mkdir -p "${STATE_DIR}"
  printf '0\n' > "${PEAK_FILE}"
  : > "${CPU_FILE}"
  : > "${CPU_LAST_FILE}"
  date +%s > "${START_FILE}"
  # Seed the minimum with a value no real volume can exceed, so the first
  # sample always wins.
  printf '%s\n' "$((1 << 60))" > "${FREE_FILE}"
  (
    while :; do
      sample_once
      sleep "${INTERVAL}"
    done
  ) &
  printf '%s\n' "$!" > "${PID_FILE}"
  printf 'resource sampler: started, interval %ss, state %s\n' \
    "${INTERVAL}" "${STATE_DIR}"
}

# Stop the background loop and wait for it to go. Killing it and reading
# immediately races the loop's own read-modify-write: the report can catch a
# state file mid-rewrite, read an empty value, and then fail on arithmetic
# under `set -u`. Waiting is the only way to be sure no writer remains.
stop_sampler() {
  local pid waited
  [ -f "${PID_FILE}" ] || return 0
  pid="$(cat "${PID_FILE}")"
  [ -n "${pid}" ] || return 0
  kill "${pid}" 2>/dev/null || true
  # `wait` applies only to a child of this shell, which the loop is when
  # `report` runs in the same process as `start`. It is not when they are
  # separate steps, so poll for the process to disappear as well.
  wait "${pid}" 2>/dev/null || true
  waited=0
  while kill -0 "${pid}" 2>/dev/null; do
    sleep 1
    waited=$((waited + 1))
    if [ "${waited}" -ge 15 ]; then
      printf 'resource sampler: sampler %s did not exit; killing\n' "${pid}"
      kill -9 "${pid}" 2>/dev/null || true
      break
    fi
  done
  rm -f "${PID_FILE}"
}

# Summarize the per-interval lines: time-weighted average (sum of busy over sum
# of total), p95 and maximum of the per-interval busy cores, the fraction of
# intervals above 90% busy, and steal as a share of all jiffies.
cpu_summary() {
  local cores
  cores="$(nproc)"
  if [ ! -s "${CPU_FILE}" ]; then
    printf 'nproc %s, no cpu samples' "${cores}"
    return 0
  fi
  # mawk (the Ubuntu default) has no asort, so sort the per-interval figures
  # with sort(1) and read the percentile back from the sorted list.
  local sorted totals
  sorted="${STATE_DIR}/cpu-sorted"
  awk -v cores="${cores}" '{ print $1 / $2 * cores }' "${CPU_FILE}" \
    | sort -n > "${sorted}"
  totals="$(awk '{ sb += $1; st += $2; ss += $3; if ($1 / $2 > 0.9) hi++ }
    END { printf "%d %d %d %d %d", NR, sb, st, ss, hi }' "${CPU_FILE}")"
  awk -v cores="${cores}" -v totals="${totals}" '
    { v[NR] = $1 }
    END {
      split(totals, t, " ")
      n = NR
      i = int(0.95 * n); if (i < 0.95 * n) i++
      if (i < 1) i = 1; if (i > n) i = n
      printf "nproc %d, samples %d, busy cores avg %.2f p95 %.2f max %.2f, "\
             "above 90%% busy %.1f%%, steal %.2f%%",
        cores, n, t[2] / t[3] * cores, v[i], v[n], 100 * t[5] / n, 100 * t[4] / t[3]
    }' "${sorted}"
}

# Overall compiler-cache hit rate, to label a run warm or cold. The first
# "Cache hits rate" line is the aggregate; per-language lines follow it.
sccache_hit_rate() {
  if ! command -v sccache > /dev/null 2>&1; then
    printf 'n/a (no sccache)'
    return 0
  fi
  sccache --show-stats 2> /dev/null \
    | awk '/^Cache hits rate/ && !seen {seen = 1; print $4 " %"} END {if (!seen) print "n/a"}'
}

report_sampler() {
  stop_sampler
  if [ ! -f "${PEAK_FILE}" ] || [ ! -f "${FREE_FILE}" ]; then
    printf 'resource sampler: no samples recorded\n'
    return 0
  fi
  # Take one final sample, so a job that finishes inside a single interval
  # still reports a real figure rather than the seed values. No writer remains
  # by this point, so this cannot race.
  sample_once
  local peak_mib free_mib total_mib
  peak_mib="$(( $(cat "${PEAK_FILE}") / 1024 ))"
  free_mib="$(( $(cat "${FREE_FILE}") / 1024 ))"
  total_mib="$(( $(awk '/^MemTotal:/ {print $2}' /proc/meminfo) / 1024 ))"
  printf 'resource sampler: peak memory used %s MiB of %s MiB\n' \
    "${peak_mib}" "${total_mib}"
  printf 'resource sampler: minimum free disk %s MiB\n' "${free_mib}"
  df -h .
  local cpu_line wall ratio
  cpu_line="$(cpu_summary)"
  wall="$(( $(date +%s) - $(cat "${START_FILE}") ))"
  ratio="$(sccache_hit_rate)"
  printf 'resource sampler: wall %ss (sampler lifetime), %s\n' "${wall}" "${cpu_line}"
  printf 'resource sampler: sccache hit rate %s\n' "${ratio}"
  if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
    {
      printf '### Resource sampler (%s)\n\n' "${GITHUB_JOB:-job}"
      printf '| Measure | Value |\n| --- | --- |\n'
      printf '| Peak memory used | %s MiB of %s MiB |\n' \
        "${peak_mib}" "${total_mib}"
      printf '| Minimum free disk | %s MiB |\n' "${free_mib}"
      printf '| Sampler lifetime | %s s |\n' "${wall}"
      printf '| CPU | %s |\n' "${cpu_line}"
      printf '| sccache hit rate | %s |\n' "${ratio}"
    } >> "${GITHUB_STEP_SUMMARY}"
  fi
}

case "${1:-}" in
  start) start_sampler ;;
  report) report_sampler ;;
  *)
    printf 'usage: %s {start|report}\n' "${0##*/}" >&2
    exit 2
    ;;
esac
