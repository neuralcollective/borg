#!/usr/bin/env bash
set -euo pipefail

# End-to-end agent entrypoint:
# 1) Terraform apply (hybrid stack)
# 2) App deploy to provisioned host
# 3) Optional settings apply + preflight (handled by agent-deploy.sh)
#
# Optional env:
#   TFVARS_FILE=/abs/path/to/terraform.tfvars
#   BORG_SETTINGS_FILE=/abs/path/to/settings.json

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
TF_DIR="${SCRIPT_DIR}/terraform/hybrid"
TFVARS_FILE="${TFVARS_FILE:-${TF_DIR}/terraform.tfvars}"

"${SCRIPT_DIR}/provision-hybrid.sh" apply

SERVER_IP="$(terraform -chdir="${TF_DIR}" output -raw server_ipv4)"
TUNNEL_TOKEN="$(terraform -chdir="${TF_DIR}" output -raw cloudflare_tunnel_token 2>/dev/null || true)"

export BORG_HOST="root@${SERVER_IP}"
if [[ -n "${TUNNEL_TOKEN}" ]]; then
  export CF_TUNNEL_TOKEN="${TUNNEL_TOKEN}"
fi

if [[ -n "${BORG_SETTINGS_FILE:-}" ]]; then
  export BORG_SETTINGS_FILE
fi

"${ROOT_DIR}/deploy/agent-deploy.sh"
