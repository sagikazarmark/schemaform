#!/usr/bin/env bash
set -euo pipefail

profile="${1:-pr}"
requested_target="${2:-}"

case "$profile" in
  pr) seconds=60 ;;
  nightly) seconds=900 ;;
  release) seconds=7200 ;;
  *)
    printf 'unknown fuzz profile: %s\n' "$profile" >&2
    exit 2
    ;;
esac

targets=(resource_compilation ui_schema_compilation form_construction uri_pointer user_commands host_transactions external_findings)
if [[ -n "$requested_target" ]]; then
  case "$requested_target" in
    resource_compilation|ui_schema_compilation|form_construction|uri_pointer|user_commands|host_transactions|external_findings)
      targets=("$requested_target")
      ;;
    *)
      printf 'unknown fuzz target: %s\n' "$requested_target" >&2
      exit 2
      ;;
  esac
fi

for target in "${targets[@]}"; do
  cargo +nightly-2026-07-23 fuzz run "$target" "fuzz/corpus/$target" --sanitizer address -- \
    -max_total_time="$seconds" \
    -timeout=10 \
    -rss_limit_mb=4096 \
    -max_len=65536 \
    -print_final_stats=1
done
