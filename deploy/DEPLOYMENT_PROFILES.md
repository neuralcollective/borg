# Deployment Profiles

Choose one profile and deploy with it consistently.

## Profile A: Managed Storage Bias

- Compute: EC2 (single instance) or ECS
- File storage: S3 (`storage_backend=s3`)
- Backup: S3-compatible offsite snapshots (`backup_backend=s3`)
- Queue: Postgres-backed internal orchestration first, SQS only if fanout pressure justifies it
- Search: Vespa (`search_backend=vespa`)
- Edge/TLS: CloudFront + ACM (or ALB directly)

Use this when:
- You want the strongest managed durability posture for original uploads.
- You are okay paying more for blob storage.

## Profile B: Cost-Optimized Self-Hosted (recommended)

- Compute: Hetzner VPS
- File storage: SeaweedFS via S3-compatible API (`storage_backend=s3` with custom endpoint)
- Backup: Backblaze B2 S3-compatible API (`backup_backend=s3`) with `backup_mode=active_work_only` by default
- Search: Vespa (`search_backend=vespa`)
- Queue: Postgres-backed internal orchestration

Use this when:
- You want the lowest serious storage cost without giving up scale.
- You are comfortable letting Borg manage storage/search ops and alerting.

## Practical Recommendation

- Start with **Profile B** to control cost.
- Keep storage and backup on S3-compatible APIs from day one so SeaweedFS, B2, and S3 remain configuration changes rather than product rewrites.
- Leave uploaded-source backup off by default and offer it as a paid opt-in via `backup_mode=include_uploads`.
- Move to **Profile A** only when managed blob storage/compliance posture matters more than spend.

## Required Settings Before Go-Live

- `backend=codex`
- `public_url=https://...` (must be externally reachable)
- `storage_backend` configured and tested
- `backup_backend` configured if offsite protection is required
- `search_backend=vespa`
- `project_max_bytes` sized for discovery workloads
- `pipeline_max_agents` tuned to host CPU/memory

Run preflight:

```bash
deploy/preflight.sh http://127.0.0.1:3131
```

Agent automation commands:

```bash
just infra-plan
just infra-apply
just infra-ship
```
