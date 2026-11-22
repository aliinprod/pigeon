#!/usr/bin/env bash
# Fetches one file with the pigeon CLI, checks its SHA-256 against the value
# the agent side recorded, and reports size / time / speed / route.
#
# Usage: transfer-check.sh PIGEON TICKET_FILE SHA_FILE REMOTE_PATH MODE
#   MODE: "auto" (direct if hole punching works, else relay) or "relay-only"
#         (fails unless the bytes really went through the relay).
# The ticket is read from a file and masked; it is never printed.
set -euo pipefail

pigeon=$1
ticket_file=$2
sha_file=$3
remote=$4
mode=$5

ticket=$(tr -d '\r\n' <"$ticket_file")
if [ -n "${GITHUB_ACTIONS:-}" ]; then
  echo "::add-mask::$ticket"
fi
expected=$(tr -d '\r\n ' <"$sha_file")

flags=()
case "$mode" in
  auto) ;;
  relay-only) flags=(--relay-only) ;;
  *)
    echo "unknown mode: $mode" >&2
    exit 2
    ;;
esac

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d' ' -f1
  else
    shasum -a 256 "$1" | cut -d' ' -f1
  fi
}

out="$mode-$(basename "$remote")"
rm -f "$out" "$out.part"
log="$mode.log"

start=$(date +%s%N)
"$pigeon" ${flags[@]+"${flags[@]}"} get "$ticket" "$remote" -o "$out" | tee "$log"
end=$(date +%s%N)

actual=$(sha256 "$out")
size=$(wc -c <"$out" | tr -d ' ')
route=$(sed -n 's/.*MB\/s, \([a-z]*\)), sha256.*/\1/p' "$log")
wall=$(awk -v a="$start" -v b="$end" 'BEGIN { printf "%.2f", (b - a) / 1e9 }')
mbps=$(awk -v s="$size" -v a="$start" -v b="$end" 'BEGIN { printf "%.3f", s / 1e6 / ((b - a) / 1e9) }')
match=no
if [ "$actual" = "$expected" ]; then match=yes; fi

echo "mode=$mode route=${route:-?} size=$size bytes wall=${wall}s (${mbps} MB/s incl. connect) sha256 match=$match"
echo "  expected $expected"
echo "  actual   $actual"
if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
  echo "| $mode | ${route:-?} | $size | $wall | $mbps | $match |" >>"$GITHUB_STEP_SUMMARY"
fi

if [ "$match" != yes ]; then
  echo "::error::SHA-256 mismatch in $mode mode"
  exit 1
fi
if [ "$mode" = relay-only ] && [ "$route" != relay ]; then
  echo "::error::relay-only transfer reported route '${route:-?}', expected 'relay'"
  exit 1
fi
rm -f "$out"
