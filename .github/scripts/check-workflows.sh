#!/usr/bin/env bash
# Static checks for a PUBLIC repository's workflows (SEC-25, SEC-32).
# Fails when a workflow:
#   - uses pull_request_target or workflow_run (run untrusted PR code with secrets),
#   - interpolates ${{ github.event.* }} / head_ref (attacker-controlled text) anywhere,
#   - uses a third-party action not pinned to a full 40-hex commit SHA,
#   - has no top-level `permissions:` block,
#   - passes `secrets: inherit`.
# Usage: check-workflows.sh [DIR]   (default .github/workflows)
set -euo pipefail
dir=${1:-.github/workflows}
fail=0
err() { echo "::error file=$1::$2"; fail=1; }

shopt -s nullglob
files=("$dir"/*.yml "$dir"/*.yaml)
[ "${#files[@]}" -gt 0 ] || { echo "no workflows in $dir"; exit 1; }

for f in "${files[@]}"; do
  if grep -nE '^\s*(pull_request_target|workflow_run)\s*:' "$f"; then
    err "$f" "pull_request_target / workflow_run is not allowed"
  fi
  if grep -nE '\$\{\{\s*github\.(event\.|head_ref)' "$f"; then
    err "$f" "do not interpolate github.event.* or github.head_ref; pass it through env: and quote it"
  fi
  if grep -nE '^\s*secrets:\s*inherit' "$f"; then
    err "$f" "secrets: inherit is not allowed"
  fi
  if ! grep -qE '^permissions:' "$f"; then
    err "$f" "missing top-level permissions: block"
  fi
  # uses: owner/repo[/path]@ref  -> ref must be a 40-hex SHA (local ./ actions are fine)
  while IFS= read -r line; do
    ref=$(sed -E 's/.*uses:[[:space:]]*["'\'']?[^@"'\'' ]+@([^"'\'' #]+).*/\1/' <<<"$line")
    if ! [[ "$ref" =~ ^[0-9a-f]{40}$ ]]; then
      err "$f" "action not pinned to a commit SHA: $line"
    fi
  done < <(grep -E '^\s*-?\s*uses:\s*' "$f" | grep -vE 'uses:\s*["'\'']?\./' || true)
done

if [ "$fail" -ne 0 ]; then
  echo "workflow checks failed"
  exit 1
fi
echo "workflow checks passed for ${#files[@]} files"
