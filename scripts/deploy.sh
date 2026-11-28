#!/usr/bin/env bash
# Builds Dapan and restarts its services. Run by the GitHub Actions deploy job
# after it checks out the new commit, or by hand: scripts/deploy.sh [previous-commit]
# If the build fails, the running bot keeps the old binary.
set -euo pipefail
cd "$(dirname "$0")/.."
prev="${1:-}"
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
# shellcheck disable=SC1091
source "$HOME/.cargo/env"

echo "==> Building $(git log --oneline -1)"
cargo build --release --locked

if systemctl --user is-enabled --quiet dapan-tts.service 2>/dev/null; then
  if [[ -z "$prev" ]] || ! git diff --quiet "$prev" HEAD -- tts-server; then
    echo "==> Restarting TTS server"
    systemctl --user restart dapan-tts.service
  fi
fi

echo "==> Restarting Dapan"
systemctl --user restart dapan.service
sleep 15
if ! systemctl --user is-active --quiet dapan.service; then
  echo "Dapan is not running after restart:" >&2
  journalctl --user -u dapan.service -n 60 --no-pager >&2 || true
  exit 1
fi
echo "==> Dapan is running"
