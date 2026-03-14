#!/usr/bin/env bash
set -euo pipefail

# One-command remote bootstrap + deploy for agent-driven operations.
# Required:
#   BORG_HOST=root@<ip-or-host>
# Optional:
#   BORG_REMOTE_DIR=/opt/borg
#   BORG_SETTINGS_FILE=/abs/path/to/settings.json
#   BORG_SERVICE_NAME=borg
#   CF_TUNNEL_TOKEN=<cloudflare tunnel token>

HOST="${BORG_HOST:?BORG_HOST is required (example: root@1.2.3.4)}"
REMOTE_DIR="${BORG_REMOTE_DIR:-/opt/borg}"
SERVICE_NAME="${BORG_SERVICE_NAME:-borg}"
SETTINGS_FILE="${BORG_SETTINGS_FILE:-}"
CF_TUNNEL_TOKEN="${CF_TUNNEL_TOKEN:-}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "missing required command: $1" >&2
    exit 1
  }
}

need_cmd ssh
need_cmd rsync
need_cmd curl
need_cmd jq

if [[ -n "${SETTINGS_FILE}" && ! -f "${SETTINGS_FILE}" ]]; then
  echo "BORG_SETTINGS_FILE does not exist: ${SETTINGS_FILE}" >&2
  exit 1
fi

echo "==> [1/6] Bootstrap host packages and runtimes on ${HOST}"
ssh "${HOST}" 'bash -s' <<'REMOTE_BOOTSTRAP'
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive

if command -v apt-get >/dev/null 2>&1; then
  apt-get update -y
  apt-get install -y \
    ca-certificates curl git rsync jq unzip \
    build-essential pkg-config libssl-dev \
    postgresql-client
  if ! command -v docker >/dev/null 2>&1; then
    apt-get install -y docker.io
  fi
  # cloudflared is not in standard repos; install only if CF_TUNNEL_TOKEN is set
  if [[ -n "${CF_TUNNEL_TOKEN:-}" ]] && ! command -v cloudflared >/dev/null 2>&1; then
    curl -fsSL https://pkg.cloudflare.com/cloudflare-main.gpg | tee /usr/share/keyrings/cloudflare-main.gpg >/dev/null
    echo 'deb [signed-by=/usr/share/keyrings/cloudflare-main.gpg] https://pkg.cloudflare.com/cloudflared noble main' > /etc/apt/sources.list.d/cloudflared.list
    apt-get update -y && apt-get install -y cloudflared
  fi
else
  echo "only apt-based hosts are currently supported by agent-deploy.sh" >&2
  exit 1
fi

systemctl enable docker >/dev/null 2>&1 || true
systemctl start docker >/dev/null 2>&1 || true

if ! command -v rustup >/dev/null 2>&1; then
  curl https://sh.rustup.rs -sSf | sh -s -- -y
fi

if ! command -v bun >/dev/null 2>&1; then
  curl -fsSL https://bun.sh/install | bash
fi
REMOTE_BOOTSTRAP

echo "==> [2/6] Sync repository to ${HOST}:${REMOTE_DIR}"
ssh "${HOST}" "mkdir -p ${REMOTE_DIR}"
rsync -az --delete \
  --exclude .git \
  --exclude node_modules \
  --exclude target \
  --exclude store \
  --exclude .env \
  --exclude '.worktrees' \
  "${ROOT_DIR}/" "${HOST}:${REMOTE_DIR}/"

# Invalidate cargo cache so changed sources are always recompiled.
# rsync preserves mtime from the local machine which can be older than
# the remote target/ artifacts, causing cargo to skip recompilation.
ssh "${HOST}" "find ${REMOTE_DIR}/borg-rs/crates -name '*.rs' -exec touch {} +"

echo "==> [3/6] Ensure .env exists on remote host"
ssh "${HOST}" "if [ ! -f ${REMOTE_DIR}/.env ]; then cp ${REMOTE_DIR}/.env.example ${REMOTE_DIR}/.env || true; fi"

echo "==> [4/6] Build and restart service"
ssh "${HOST}" "bash -s" <<REMOTE_BUILD
set -euo pipefail
export PATH="\$HOME/.cargo/bin:\$HOME/.bun/bin:\$PATH"
cd ${REMOTE_DIR}

cd borg-rs && cargo build --release && cd ..
cd dashboard && bun install --frozen-lockfile && bun run build && cd ..
cd sidecar/agent-bridge && bun install && cd ../..
docker build -t borg-agent -f container/Dockerfile container/

# Ensure borg user owns the entire working directory
if id borg >/dev/null 2>&1; then
  chown -R borg:borg ${REMOTE_DIR}/borg-rs/target/release/borg-server
  chown -R borg:borg ${REMOTE_DIR}/dashboard/dist
  chown -R borg:borg ${REMOTE_DIR}/store 2>/dev/null || true
  # Install global CLI tools the service needs at runtime
  su - borg -c "bun install -g @openai/codex@latest" 2>/dev/null || true
fi

# Restart the borg user service (preferred) or fall back to system service
if id borg >/dev/null 2>&1; then
  BORG_UID=\$(id -u borg)
  su - borg -c "export XDG_RUNTIME_DIR=/run/user/\${BORG_UID} && systemctl --user daemon-reload && systemctl --user restart ${SERVICE_NAME}" && echo "restarted user service" || {
    echo "user service restart failed, falling back to system service"
    cp deploy/borg.service /etc/systemd/system/${SERVICE_NAME}.service
    systemctl daemon-reload
    systemctl restart ${SERVICE_NAME}
  }
else
  cp deploy/borg.service /etc/systemd/system/${SERVICE_NAME}.service
  cp deploy/borg.socket /etc/systemd/system/${SERVICE_NAME}.socket
  systemctl daemon-reload
  systemctl enable ${SERVICE_NAME}.socket
  systemctl start ${SERVICE_NAME}.socket
  systemctl restart ${SERVICE_NAME}
fi

if [[ -n "${CF_TUNNEL_TOKEN}" ]]; then
  cloudflared service install "${CF_TUNNEL_TOKEN}" || true
  systemctl enable cloudflared >/dev/null 2>&1 || true
  systemctl restart cloudflared >/dev/null 2>&1 || true
fi
REMOTE_BUILD

if [[ -n "${SETTINGS_FILE}" ]]; then
  echo "==> [5/6] Apply settings from ${SETTINGS_FILE}"
  tmp_remote="/tmp/borg-settings-$(date +%s).json"
  scp "${SETTINGS_FILE}" "${HOST}:${tmp_remote}" >/dev/null
  ssh "${HOST}" "bash -s" <<REMOTE_SETTINGS
set -euo pipefail
token=\$(curl -fsS http://127.0.0.1:3131/api/auth/token | jq -r '.token // empty')
if [ -z "\${token}" ]; then
  echo "failed to fetch auth token from local API" >&2
  exit 1
fi
curl -fsS \
  -H "Authorization: Bearer \${token}" \
  -H "Content-Type: application/json" \
  -X PUT \
  --data-binary @${tmp_remote} \
  http://127.0.0.1:3131/api/settings >/dev/null
rm -f ${tmp_remote}
REMOTE_SETTINGS
else
  echo "==> [5/6] Skip settings apply (BORG_SETTINGS_FILE not set)"
fi

echo "==> [6/6] Run remote preflight"
ssh "${HOST}" "bash -s" <<REMOTE_PREFLIGHT
set -euo pipefail
cd ${REMOTE_DIR}
deploy/preflight.sh http://127.0.0.1:3131
REMOTE_PREFLIGHT

echo
echo "Deploy complete."
