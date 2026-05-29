#!/usr/bin/env sh
set -eu

ROOT="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
DEPLOY=0
LIVE=0

usage() {
  cat <<'EOF'
usage: sh tools/verify.sh [--deploy] [--live]

Runs the continuous Zeta verification chain.

Default checks:
  cargo fmt --check
  cargo test
  python3 tools/check-docs.py
  python3 tools/check-vscode-extension.py
  sh tools/build-website.sh
  git diff --check

Options:
  --deploy  deploy website after local verification
  --live    run live website smoke after deploy/build

Live smoke requires Playwright. Either install it locally where Node can
resolve it, or set ZETA_PLAYWRIGHT_REQUIRE to a playwright package path.
EOF
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --deploy)
      DEPLOY=1
      ;;
    --live)
      LIVE=1
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unknown option: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
  shift
done

step() {
  printf '\n==> %s\n' "$1"
}

cd "$ROOT"

step "cargo fmt --check"
cargo fmt --check

step "cargo test"
cargo test

step "python3 tools/check-docs.py"
python3 tools/check-docs.py

step "python3 tools/check-vscode-extension.py"
python3 tools/check-vscode-extension.py

if [ "$DEPLOY" -eq 1 ]; then
  step "sh tools/deploy-website.sh"
  sh tools/deploy-website.sh
else
  step "sh tools/build-website.sh"
  sh tools/build-website.sh
fi

if [ "$LIVE" -eq 1 ]; then
  step "node tools/smoke-website-live.cjs"
  node tools/smoke-website-live.cjs
fi

step "git diff --check"
git diff --check

printf '\nZeta verification passed.\n'
