#!/usr/bin/env bash
# Runs the Playwright legs in the image CI uses, so the screenshot baselines under
# tests/visual/__screenshots__ are captured with the same browser and fonts.
#
#   bun run --cwd web test:visual:container            # compare
#   bun run --cwd web test:visual:container --update-snapshots
#
# Needs podman or docker; the repository is mounted read-write at /work.
set -euo pipefail
image="mcr.microsoft.com/playwright:v1.64.1-noble"
root="$(cd "$(dirname "$0")/../.." && pwd)"
runtime="$(command -v podman || command -v docker)"
exec "$runtime" run --rm --network host -v "$root":/work:Z -w /work/web -e CI=1 -e HOME=/tmp/home \
  "$image" bash -lc "mkdir -p /tmp/home && npx playwright test --project visual --project phone --project a11y $*"
