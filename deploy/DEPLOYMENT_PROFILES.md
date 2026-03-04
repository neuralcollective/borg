# Deployment Profiles

Choose one profile and deploy with it consistently.

## Profile A: AWS-Only (easiest operations)

- Compute: EC2 (single instance) or ECS
- File storage: S3 (`storage_backend=s3`)
- Queue: SQS (`ingestion_queue_backend=sqs`)
- Search: OpenSearch (`search_backend=opensearch`) for large corpora
- Edge/TLS: CloudFront + ACM (or ALB directly)

Use this when:
- You want one cloud/vendor and fastest path to production operations.
- You are okay paying more for managed services.

## Profile B: Low-Cost Hybrid (recommended cost/perf)

- Compute: Hetzner VPS
- Edge/TLS/SSO: Cloudflare Tunnel + Access
- File storage: S3-compatible object storage (Cloudflare R2 or AWS S3)
- Queue/Search: start local/in-memory, add AWS SQS/OpenSearch only when needed

Use this when:
- You want much lower monthly baseline cost.
- You still need robust, resumable, large-file ingest.

## Practical Recommendation

- Start with **Profile B** to control cost.
- Keep storage on S3-compatible API from day one (`storage_backend=s3`) so migration between S3/R2 is mostly config-only.
- Move to **Profile A** only when managed-service ops simplicity matters more than spend.

## Required Settings Before Go-Live

- `backend=codex`
- `public_url=https://...` (must be externally reachable)
- `storage_backend` configured and tested
- `project_max_bytes` sized for discovery workloads
- `pipeline_max_agents` tuned to host CPU/memory

Run preflight:

```bash
deploy/preflight.sh http://127.0.0.1:3131
```
