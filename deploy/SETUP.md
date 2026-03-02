# Borg Deployment Setup Guide

Complete setup for: `borg.legal` (landing), `app.borg.legal` (dashboard), `api.borg.legal` (backend on Hetzner).

## Architecture

```
                    Cloudflare DNS (borg.legal zone)
                    ┌──────────────────────────────────┐
                    │  borg.legal     → Workers         │
                    │  app.borg.legal → Workers         │
                    │  api.borg.legal → Hetzner VPS     │
                    └──────────────────────────────────┘
                              │              │
                    ┌─────────┘              └─────────┐
                    ▼                                   ▼
          Cloudflare Workers                    Hetzner VPS (CAX21)
          (Static Assets)                       ┌─────────────────┐
          ┌────────────────┐                    │  Caddy (TLS)    │
          │ landing/       │                    │    ↓             │
          │ dashboard/dist │                    │  borg-server    │
          └────────────────┘                    │    :3131        │
                                                └─────────────────┘
```

**Cost**: ~€6.49/mo (Hetzner) + free (Cloudflare). Hetzner prices rise 30-37% on April 1, 2026 — provision before then.

---

## Part 1: DNS (Porkbun → Cloudflare)

Cloudflare requires its own nameservers for apex domain (`borg.legal`) to work with Workers/Pages. Domain stays registered at Porkbun.

### 1.1 Add zone to Cloudflare

1. [dash.cloudflare.com](https://dash.cloudflare.com) → **Add a Site** → enter `borg.legal` → Free plan
2. Cloudflare assigns two nameservers (e.g. `adam.ns.cloudflare.com`, `lara.ns.cloudflare.com`). Copy both.

### 1.2 Update Porkbun nameservers

1. [porkbun.com](https://porkbun.com) → Domains → `borg.legal` → Details → Nameservers
2. Remove Porkbun defaults, add Cloudflare's two nameservers
3. If DNSSEC is enabled on Porkbun, disable it first (conflicts with Cloudflare)

Propagation: usually 30 min – 2 hours. Cloudflare emails when zone goes active.

### 1.3 Add API record

Once zone is active, go to **Cloudflare DNS → Records**:

| Type | Name  | Value             | Proxy   |
|------|-------|-------------------|---------|
| A    | `api` | `<Hetzner VPS IP>` | Proxied |

Do NOT add records for `borg.legal` or `app.borg.legal` — Workers custom domains create these automatically.

---

## Part 2: Hetzner VPS

### 2.1 Install hcloud CLI

```bash
# Arch
sudo pacman -S hcloud-cli

# Or manual
curl -sSL https://github.com/hetznercloud/cli/releases/latest/download/hcloud-linux-amd64.tar.gz | tar xz
sudo mv hcloud /usr/local/bin/
```

```bash
hcloud context create borg
# Paste API token from console.hetzner.cloud → Project → API Tokens
```

### 2.2 One-shot provisioning

Save as `deploy/cloud-init.yml`:

```yaml
#cloud-config
package_update: true
package_upgrade: true

packages:
  - fail2ban
  - ufw

users:
  - name: deploy
    groups: docker, sudo
    shell: /bin/bash
    sudo: ['ALL=(ALL) NOPASSWD:ALL']
    lock_passwd: true
    ssh_authorized_keys:
      - ssh-ed25519 AAAA... your-key-here

ssh_pwauth: false
disable_root: true

runcmd:
  - curl -fsSL https://get.docker.com | sh
  - usermod -aG docker deploy
  - ufw allow ssh && ufw allow http && ufw allow https && ufw --force enable
  - systemctl enable fail2ban && systemctl start fail2ban
  - mkdir -p /opt/borg && chown deploy:deploy /opt/borg
```

Then provision:

```bash
# Upload SSH key
hcloud ssh-key create --name borg-key --public-key-from-file ~/.ssh/id_ed25519.pub

# Create firewall
hcloud firewall create --name borg-fw
for port in 22 80 443; do
  hcloud firewall add-rule borg-fw \
    --direction in --protocol tcp --port $port \
    --source-ips 0.0.0.0/0 --source-ips ::/0
done

# Create server (~30s to provision, ~2-3min for cloud-init)
hcloud server create \
  --name borg \
  --type cax21 \
  --image ubuntu-24.04 \
  --location nbg1 \
  --ssh-key borg-key \
  --firewall borg-fw \
  --user-data-from-file deploy/cloud-init.yml

# Get IP
hcloud server ip borg
```

**Server choice**: `cax21` = 4 ARM vCPU, 8 GB RAM, 80 GB disk, €6.49/mo. Use `cx33` (Intel) if you need x86 compat — same specs, €5.49/mo.

### 2.3 Deploy borg

Once cloud-init finishes (~3 min after creation):

```bash
ssh deploy@<IP>

# Clone repo
git clone https://github.com/your-org/borg.git /opt/borg
cd /opt/borg/deploy

# Create .env with your secrets
cat > .env << 'EOF'
TELEGRAM_TOKEN=...
ANTHROPIC_API_KEY=...
BORG_DB_PATH=/app/store/borg.db
ALLOWED_ORIGINS=https://borg.legal,https://app.borg.legal
EOF

# Build and start
docker compose build borg
docker compose up -d

# Verify
curl http://localhost:3131/api/status
```

### 2.4 Systemd auto-start (optional)

```bash
sudo cp /opt/borg/deploy/borg.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable borg
```

### 2.5 Enable backups

```bash
hcloud server enable-backup borg   # +20% cost, automated daily snapshots
```

---

## Part 3: Frontend on Cloudflare Workers (Static Assets)

Cloudflare deprecated Pages (April 2025) in favor of Workers with Static Assets. Workers gives full CLI automation — no dashboard required for deploys.

### 3.1 Install wrangler

```bash
cd dashboard && bun add -D wrangler
```

### 3.2 Dashboard worker config

Create `dashboard/wrangler.toml`:

```toml
name = "borg-dashboard"
compatibility_date = "2026-03-01"

[assets]
directory = "./dist"
not_found_handling = "single-page-application"
```

### 3.3 Landing page worker config

Create `landing/wrangler.toml`:

```toml
name = "borg-landing"
compatibility_date = "2026-03-01"

[assets]
directory = "."
```

### 3.4 Build and deploy

```bash
# First time — login to Cloudflare
bunx wrangler login

# Dashboard
cd dashboard
BORG_API_URL=https://api.borg.legal bash build.sh
bunx wrangler deploy

# Landing
cd ../landing
bunx wrangler deploy
```

First deploy creates the Worker project automatically.

### 3.5 Custom domains

After deploying, attach custom domains via wrangler.toml routes:

**Dashboard** (`dashboard/wrangler.toml`):
```toml
routes = [
  { pattern = "app.borg.legal", custom_domain = true }
]
```

**Landing** (`landing/wrangler.toml`):
```toml
routes = [
  { pattern = "borg.legal", custom_domain = true }
]
```

Then `bunx wrangler deploy` again — this wires up DNS automatically since the zone is on your Cloudflare account. TLS certificates are issued automatically.

### 3.6 CI/CD with GitHub Actions (optional)

Create `.github/workflows/deploy-frontend.yml`:

```yaml
name: Deploy Frontend
on:
  push:
    branches: [main]
    paths:
      - 'dashboard/**'
      - 'landing/**'

jobs:
  dashboard:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: oven-sh/setup-bun@v2
      - run: cd dashboard && bun install && BORG_API_URL=https://api.borg.legal bash build.sh
      - uses: cloudflare/wrangler-action@v3
        with:
          apiToken: ${{ secrets.CLOUDFLARE_API_TOKEN }}
          accountId: ${{ secrets.CLOUDFLARE_ACCOUNT_ID }}
          workingDirectory: dashboard
          command: deploy

  landing:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: cloudflare/wrangler-action@v3
        with:
          apiToken: ${{ secrets.CLOUDFLARE_API_TOKEN }}
          accountId: ${{ secrets.CLOUDFLARE_ACCOUNT_ID }}
          workingDirectory: landing
          command: deploy
```

Set `CLOUDFLARE_API_TOKEN` and `CLOUDFLARE_ACCOUNT_ID` as GitHub Actions secrets.

---

## Part 4: Quick Reference

### Full deploy sequence (from zero)

```
1. Add borg.legal zone to Cloudflare           (~1 min)
2. Update Porkbun nameservers                   (~2 min)
3. Wait for NS propagation                      (~30 min - 2 hrs)
4. hcloud server create                         (~3 min)
5. SSH in, clone, docker compose up             (~5 min)
6. Add api.borg.legal A record in Cloudflare    (~1 min)
7. wrangler deploy (dashboard + landing)        (~2 min)
8. Add custom_domain routes, redeploy           (~1 min)
```

Total: ~45 min active work + NS propagation wait.

### Useful commands

```bash
# Check borg server health
curl https://api.borg.legal/api/status | jq

# Redeploy borg backend
BORG_HOST=deploy@<IP> bash deploy/deploy.sh

# Redeploy dashboard
cd dashboard && BORG_API_URL=https://api.borg.legal bash build.sh && bunx wrangler deploy

# SSH to server
ssh deploy@$(hcloud server ip borg)

# View borg logs
ssh deploy@$(hcloud server ip borg) 'cd /opt/borg/deploy && docker compose logs -f borg'

# Hetzner server status
hcloud server describe borg
```

### Secrets needed

| Where | Secret |
|-------|--------|
| Hetzner `.env` | `TELEGRAM_TOKEN`, `ANTHROPIC_API_KEY`, plus any others from borg config |
| GitHub Actions | `CLOUDFLARE_API_TOKEN`, `CLOUDFLARE_ACCOUNT_ID` |
| Cloudflare | API token with Workers permissions (create at dash.cloudflare.com → API Tokens) |

### Cost breakdown

| Service | Monthly |
|---------|---------|
| Hetzner CAX21 (4 vCPU, 8 GB) | €6.49 (€8.40 after April 2026) |
| Hetzner backups (+20%) | €1.30 |
| Cloudflare Workers (free tier) | €0 |
| Porkbun borg.legal renewal | ~$7/yr |
| **Total** | **~€8/mo** |
