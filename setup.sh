#!/usr/bin/env bash
set -euo pipefail

# Install Rust if not present
if ! command -v cargo &>/dev/null; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
  source "$HOME/.cargo/env"
fi

# Install sqlite3 dev headers (needed by libsqlite3-sys) and the sqlite3 CLI
# (used by the deploy workflow's pre-launch DB reset guard).
if command -v apt-get &>/dev/null; then
  apt-get update -qq && apt-get install -y -qq libsqlite3-dev sqlite3 pkg-config
fi

cd /home/exedev/sydney.tylerrouze.com

# Build release binary
cargo build --release

# Install a systemd service so the app restarts on reboot
cat > /etc/systemd/system/wedding.service <<EOF
[Unit]
Description=Sydney's Wedding Site
After=network.target

[Service]
User=exedev
WorkingDirectory=/home/exedev/sydney.tylerrouze.com
ExecStart=/home/exedev/sydney.tylerrouze.com/target/release/wedding-rsvp
Restart=on-failure
# App config (ADMIN_TOKEN, LISTMONK_*, optional DATABASE_URL) is written here by
# the deploy workflow from GitHub Secrets/Variables. Leading '-' = optional, so a
# fresh VM still boots before the first deploy creates the file. DATABASE_URL
# defaults to sqlite:data/wedding.db in-app when unset.
EnvironmentFile=-/home/exedev/sydney.tylerrouze.com/wedding.env
Environment=RUST_LOG=wedding_rsvp=info,tower_http=info

[Install]
WantedBy=multi-user.target
EOF

systemctl daemon-reload
systemctl enable --now wedding
echo "Setup complete. Site running on port 8080."
