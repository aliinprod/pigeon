#!/usr/bin/env bash
# Waits until an artifact named NAME exists in THIS workflow run, then
# downloads and extracts it into DEST.
#
# Lets two jobs that run in parallel hand data to each other: one uploads with
# actions/upload-artifact@v4 (v4 artifacts are visible in the REST API as soon
# as the upload step finishes, while the run is still going), the other polls
# here.
#
# Usage: wait-artifact.sh NAME DEST [TIMEOUT_SECS]
# Needs: gh, python (for unzip), GH_TOKEN with `actions: read`,
#        GITHUB_REPOSITORY and GITHUB_RUN_ID (set by Actions).
# Never prints the artifact contents.
set -euo pipefail

name=$1
dest=$2
timeout=${3:-600}
repo=${GITHUB_REPOSITORY:?GITHUB_REPOSITORY not set}
run=${GITHUB_RUN_ID:?GITHUB_RUN_ID not set}

# The Windows "python3" in PATH can be a Store stub that only prints a hint.
if python3 -c "" >/dev/null 2>&1; then py=python3; else py=python; fi

deadline=$(($(date +%s) + timeout))
id=""
while :; do
  id=$(gh api "repos/$repo/actions/runs/$run/artifacts?name=$name&per_page=1" \
    --jq '.artifacts[0].id // empty' 2>/dev/null || true)
  if [ -n "$id" ]; then
    break
  fi
  if [ "$(date +%s)" -ge "$deadline" ]; then
    echo "::error::artifact '$name' did not appear within ${timeout}s"
    exit 1
  fi
  echo "waiting for artifact '$name'..."
  sleep 10
done

mkdir -p "$dest"
zip="$dest/.artifact-$id.zip"
# The zip endpoint redirects to signed blob storage; gh follows it.
gh api "repos/$repo/actions/artifacts/$id/zip" >"$zip"
"$py" -c 'import sys, zipfile; zipfile.ZipFile(sys.argv[1]).extractall(sys.argv[2])' "$zip" "$dest"
rm -f "$zip"
echo "artifact '$name' (id $id) extracted to $dest"
