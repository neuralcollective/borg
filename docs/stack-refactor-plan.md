# Borg Stack Refactor Plan

## Mission

Build Borg around the strongest long-term architecture for very large legal and repo corpora:

- `Postgres` for the control plane
- `Vespa` for external retrieval
- `SeaweedFS` for primary blob storage via S3-compatible APIs
- `Backblaze B2` or another S3-compatible target for offsite backup snapshots
- Borg-managed ops automation, health checks, repair, and alerting

This is a clean-break refactor. SQLite is not the target architecture.

## Decisions Locked In

### Control plane

- `Postgres` is the system of record.
- Workflow state, tasks, reviews, uploads, audit events, and settings belong there.

### Retrieval

- `Vespa` is the only external search backend.
- Borg owns the retrieval layer above it.
- SQLite FTS and OpenSearch are not part of the target architecture.

### Storage

- Primary storage is S3-compatible, optimized for `SeaweedFS`.
- Backup defaults to `active_work_only`.
- Raw uploaded source-file backup is opt-in via `backup_mode=include_uploads`.

### Operations

- Borg should monitor and operate its own stack.
- Health and backup status must be visible through the API.

## Completed Slices

1. External search moved to a `Vespa` client path and SQLite fallback retrieval was removed from the server request path.
2. Backup policy/config was added with `active_work_only` as the default and `include_uploads` as the paid/explicit path.
3. Offsite backup snapshots for active work were added through an S3-compatible backup target.
4. Local dependency stack files were added for `Postgres + SeaweedFS + Vespa`.
5. A local ingest/retrieval load harness was added to exercise chunked ZIP ingestion and query-time retrieval.

## Remaining Critical Work

1. Replace the SQLite-bound `Db` layer in `borg-core` with a Postgres-native implementation.
2. Remove SQLite schema/migration/runtime assumptions entirely.
3. Replace remaining SQLite-only indexing/embedding storage assumptions where they conflict with the target architecture.
4. Validate the local stack end-to-end with real ingest and retrieval runs.
5. Add deeper ops/repair automation once the Postgres control plane is in place.

## Guardrails

- Do not add new feature work that deepens SQLite coupling.
- Do not reintroduce a fallback path from Vespa to SQLite FTS.
- Keep all blob storage flows S3-compatible so SeaweedFS, B2, and S3 remain interchangeable at the API boundary.
