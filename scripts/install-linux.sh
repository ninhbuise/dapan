#!/usr/bin/env bash
# Installs Dapan as systemd user services (start at boot, restart on crash).
# Needs once, as root:  sudo loginctl enable-linger "$USER"
# Usage: scripts/install-linux.sh
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
units="$HOME/.config/systemd/user"
mkdir -p "$units"

install_unit() {
  sed "s|@ROOT@|$root|g" "$root/deploy/systemd/$1" > "$units/$1"
  echo "installed $units/$1"
}

install_unit dapan.service
enable=(dapan.service)
if [[ -x "$root/tts-server/.venv/bin/python" ]]; then
  install_unit dapan-tts.service
  enable+=(dapan-tts.service)
fi

systemctl --user daemon-reload
systemctl --user enable "${enable[@]}"

[[ -f "$root/.env" ]] || echo "Note: $root/.env is missing; copy .env.example and fill it in."
[[ -x "$root/target/release/dapan" ]] || echo "Note: not built yet; run scripts/deploy.sh."
loginctl show-user "$USER" -p Linger | grep -q yes \
  || echo "Note: linger is off; run: sudo loginctl enable-linger $USER"
