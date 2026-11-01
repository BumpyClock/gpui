#!/bin/sh
set -eu
cd "$(git rev-parse --show-toplevel)"
[ -x .githooks/pre-push ] || { echo 'Tracked pre-push is missing or not executable.' >&2; exit 1; }
if configured=$(git config --get core.hooksPath); then
  if [ "$configured" = '.githooks' ]; then
    echo 'Tracked hooks already configured.'
    exit 0
  fi
  echo 'Refusing to replace existing core.hooksPath. Integrate hooks manually.' >&2
  exit 1
fi
hooks=$(git rev-parse --git-path hooks)
for hook in "$hooks"/*; do
  [ -f "$hook" ] && [ -x "$hook" ] || continue
  case "$hook" in *.sample) continue ;; esac
  echo "Refusing to bypass existing active hook: $hook" >&2
  exit 1
done
git config --local core.hooksPath .githooks
echo 'Tracked hooks configured. Prepare the pinned toolchain and locked dependencies before pushing.'
