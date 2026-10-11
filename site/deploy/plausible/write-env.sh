#!/usr/bin/env bash
# Write /srv/plausible/.env for Plausible CE with fresh secrets (WEBSITE.md §6.10).
# Never overwrites an existing .env: secrets must stay the same once data exists
# (TOTP_VAULT_KEY encrypts the TOTP secrets).
#
#   site/deploy/plausible/write-env.sh [/srv/plausible]
set -euo pipefail

dir=${1:-/srv/plausible}
env=$dir/.env
if [[ -e $env ]]; then
    echo "$env already exists; not touching it." >&2
    exit 1
fi
umask 077
cat > "$env" <<EOF
BASE_URL=https://plausible.harababurel.com
SECRET_KEY_BASE=$(openssl rand -base64 48 | tr -d '\n')
TOTP_VAULT_KEY=$(openssl rand -base64 32 | tr -d '\n')
HTTP_PORT=8000
# After creating your account: set this to true, then run: docker compose up -d
DISABLE_REGISTRATION=invite_only
EOF
echo "Wrote $env (mode 600). Keep a copy of it in your password manager."
