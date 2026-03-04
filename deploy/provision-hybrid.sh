#!/usr/bin/env bash
set -euo pipefail

# Agent-friendly infra provisioner for hybrid stack.
# Usage:
#   deploy/provision-hybrid.sh [plan|apply|destroy]
#
# Environment:
#   TFVARS_FILE=/abs/path/to/terraform.tfvars   (optional)
#   AUTO_APPROVE=true                            (optional, default true for apply)
#   BORG_SETTINGS_FILE=/abs/path/settings.json   (optional, for next-step deploy hint)

ACTION="${1:-apply}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
TF_DIR="${SCRIPT_DIR}/terraform/hybrid"
TFVARS_FILE="${TFVARS_FILE:-${TF_DIR}/terraform.tfvars}"
AUTO_APPROVE="${AUTO_APPROVE:-true}"
SETTINGS_FILE="${BORG_SETTINGS_FILE:-}"

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "missing required command: $1" >&2
    exit 1
  }
}

need_cmd terraform
need_cmd jq

if [[ ! -f "${TFVARS_FILE}" ]]; then
  echo "terraform vars file not found: ${TFVARS_FILE}" >&2
  echo "copy ${TF_DIR}/terraform.tfvars.example to ${TFVARS_FILE} and fill values" >&2
  exit 1
fi

cd "${TF_DIR}"
terraform init -upgrade

case "${ACTION}" in
  plan)
    terraform plan -var-file="${TFVARS_FILE}"
    ;;
  apply)
    if [[ "${AUTO_APPROVE}" == "true" ]]; then
      terraform apply -auto-approve -var-file="${TFVARS_FILE}"
    else
      terraform apply -var-file="${TFVARS_FILE}"
    fi
    ;;
  destroy)
    if [[ "${AUTO_APPROVE}" == "true" ]]; then
      terraform destroy -auto-approve -var-file="${TFVARS_FILE}"
    else
      terraform destroy -var-file="${TFVARS_FILE}"
    fi
    ;;
  *)
    echo "unknown action: ${ACTION} (expected plan|apply|destroy)" >&2
    exit 1
    ;;
esac

if [[ "${ACTION}" != "apply" ]]; then
  exit 0
fi

SERVER_IP="$(terraform output -raw server_ipv4)"
APP_URL="$(terraform output -raw app_url)"
TUNNEL_TOKEN="$(terraform output -raw cloudflare_tunnel_token 2>/dev/null || true)"
R2_BUCKET="$(terraform output -raw r2_bucket_name 2>/dev/null || true)"

echo
echo "Provision complete."
echo "Server IP: ${SERVER_IP}"
echo "App URL: ${APP_URL}"
if [[ -n "${R2_BUCKET}" ]]; then
  echo "R2 bucket: ${R2_BUCKET}"
fi

if [[ -n "${TUNNEL_TOKEN}" ]]; then
  echo
  echo "Cloudflared token captured. Store it in a secret manager and use during deploy."
fi

echo
echo "Next step (agent deploy):"
if [[ -n "${SETTINGS_FILE}" ]]; then
  echo "BORG_HOST=root@${SERVER_IP} BORG_SETTINGS_FILE=${SETTINGS_FILE} deploy/agent-deploy.sh"
else
  echo "BORG_HOST=root@${SERVER_IP} deploy/agent-deploy.sh"
fi
