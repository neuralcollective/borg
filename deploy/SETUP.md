# Borg Deployment — app.borg.legal

Hetzner VPS + Cloudflare Tunnel + Cloudflare Access (SSO).

## Architecture

```
VPS (host)
├── systemd: borg-server       API + dashboard on :3131
├── systemd: cloudflared       tunnel to Cloudflare
├── docker daemon
│   └── borg-agent containers  spawned per-task, isolated, ephemeral
├── /opt/borg/store/           SQLite DB, sessions, mirrors
└── /opt/borg/dashboard/dist/  static dashboard files
```

No ports exposed. Cloudflare handles TLS + SSO. ~€6.49/mo.

---

## Step 1: Cloudflare (~10 min)

### 1.1 Add zone + nameservers

1. [dash.cloudflare.com](https://dash.cloudflare.com) → **Add a Site** → `borg.legal` → Free plan
2. Copy the two nameservers Cloudflare assigns
3. At Porkbun → `borg.legal` → replace nameservers with Cloudflare's
4. Wait for propagation (~30 min)

### 1.2 Create Tunnel

1. **Zero Trust** → **Networks** → **Tunnels** → **Create a tunnel**
2. Name: `borg`, choose **Cloudflared** connector
3. Copy the tunnel token
4. Add public hostname:
   - Subdomain: `app`, Domain: `borg.legal`
   - Service: `http://localhost:3131`

### 1.3 Add Access Policy (SSO)

1. **Zero Trust** → **Access** → **Applications** → **Add an Application**
2. Type: **Self-hosted**, domain: `app.borg.legal`
3. Add identity providers (Google / GitHub)
4. Policy: **Allow** → Emails → your email(s)

---

## Step 2: Hetzner VPS (~5 min)

```bash
# Install hcloud CLI
sudo pacman -S hcloud-cli  # or brew install hcloud
hcloud context create borg  # paste API token from console.hetzner.cloud

# Upload SSH key
hcloud ssh-key create --name borg-key --public-key-from-file ~/.ssh/id_ed25519.pub

# Create firewall (SSH only — tunnel handles HTTP)
hcloud firewall create --name borg-fw
hcloud firewall add-rule borg-fw \
  --direction in --protocol tcp --port 22 \
  --source-ips 0.0.0.0/0 --source-ips ::/0

# Create server (~3 min for cloud-init)
hcloud server create \
  --name borg \
  --type cax21 \
  --image ubuntu-24.04 \
  --location nbg1 \
  --ssh-key borg-key \
  --firewall borg-fw \
  --user-data-from-file deploy/cloud-init.yml
```

CAX21 = 4 ARM vCPU, 8 GB RAM, 80 GB disk, €6.49/mo.

---

## Step 3: First Deploy (~10 min)

Wait ~3 min for cloud-init, then:

```bash
VPS=$(hcloud server ip borg)
ssh root@$VPS

# Clone repo
git clone https://github.com/neuralcollective/borg.git /opt/borg
cd /opt/borg

# Create .env
cat > .env << 'EOF'
CLAUDE_CODE_OAUTH_TOKEN=<your-oauth-token>
SANDBOX_BACKEND=docker
CONTAINER_IMAGE=borg-agent
CONTINUOUS_MODE=true
DATA_DIR=store
DASHBOARD_DIST_DIR=dashboard/dist
MODEL=claude-sonnet-4-6
PIPELINE_MAX_AGENTS=2
RUST_LOG=info
WEB_BIND=127.0.0.1
EOF

# Build everything
source ~/.cargo/env
cd borg-rs && cargo build --release && cd ..
cd dashboard && bun install && bun run build && cd ..
docker build -t borg-agent -f container/Dockerfile container/

# Install borg service
cp deploy/borg.service /etc/systemd/system/
systemctl daemon-reload && systemctl enable borg && systemctl start borg

# Install cloudflared tunnel
cloudflared service install <your-tunnel-token>

# Verify
curl http://127.0.0.1:3131/api/health
```

Dashboard is now live at `https://app.borg.legal` behind Cloudflare Access.

---

## Updating

```bash
# From your dev machine:
BORG_HOST=root@$(hcloud server ip borg) bash deploy/deploy.sh
```

Or manually on the VPS:
```bash
cd /opt/borg && git pull
source ~/.cargo/env && cd borg-rs && cargo build --release && cd ..
systemctl restart borg
```

## Getting your Claude OAuth token

```bash
cat ~/.claude/.credentials.json | jq -r '.oauthToken'
```

## Useful commands

```bash
# Logs
ssh root@$(hcloud server ip borg) journalctl -u borg -f

# Restart
ssh root@$(hcloud server ip borg) systemctl restart borg

# Rebuild agent image
ssh root@$(hcloud server ip borg) 'cd /opt/borg && docker build -t borg-agent -f container/Dockerfile container/'

# DB backup
scp root@$(hcloud server ip borg):/opt/borg/store/borg.db ./borg-backup.db
```
