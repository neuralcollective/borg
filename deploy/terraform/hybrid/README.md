# Hybrid Terraform Stack

This stack provisions:

- Hetzner host (Borg compute node)
- SSH-only firewall
- Cloudflare Zero Trust tunnel + DNS route to `app.<domain>`
- Optional Cloudflare Access email policy
- Optional R2 bucket

## Required API Permissions

- Hetzner token: server, firewall, SSH key manage
- Cloudflare token:
  - Zone DNS edit
  - Zero Trust tunnel/app/policy edit
  - R2 edit (if `create_r2_bucket=true`)

## Quick Start

```bash
cp terraform.tfvars.example terraform.tfvars
# edit values
terraform init
terraform apply
```

Use wrapper from repo root:

```bash
deploy/provision-hybrid.sh apply
```

## Notes

- `cloudflare_tunnel_token` output is sensitive. Store it in a secret manager.
- After provisioning, run `deploy/agent-deploy.sh` against the output host IP.
- For production, keep `storage_backend=s3` and point to either AWS S3 or R2-compatible endpoint.
