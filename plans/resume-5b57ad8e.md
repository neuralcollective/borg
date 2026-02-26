# Resume Handoff: 5b57ad8e-cc83-4000-b700-90bce0abf8db

## Completed in this continuation

### 1) Self-update path + retry behavior
- `rebuild_and_exec` now builds in `repo_path/borg-rs` and returns success bool.
- Self-update loop only advances remote-head marker after successful restart path.
- Force-restart flow now retries if rebuild fails.

Files:
- `borg-rs/crates/borg-server/src/routes.rs`
- `borg-rs/crates/borg-server/src/main.rs`

### 2) Failed phase advancement bug (critical)
- Agent phase no longer advances pipeline when backend reports `success=false`.
- Failed phases route through `fail_or_retry(...)` immediately.

File:
- `borg-rs/crates/borg-core/src/pipeline.rs`

### 3) Rebase loop hardening
- Rebase no longer auto-aborts immediately on first conflict (fix agent can work with real conflict context).
- Added explicit rebase helpers: `rebase_abort`, `rebase_in_progress`.
- Rebase phase now handles post-fix continue/abort/retry deterministically and captures latest error for retry accounting.

Files:
- `borg-rs/crates/borg-core/src/git.rs`
- `borg-rs/crates/borg-core/src/pipeline.rs`

### 4) Restart session recovery (explicit)
- Startup now explicitly logs count of active pipeline tasks with resumable `session_id` in active phases.
- Existing runtime dispatch already reuses persisted `task.session_id` via `--resume` when phase is not fresh.

File:
- `borg-rs/crates/borg-server/src/main.rs`

### 5) Service-management docs normalized (no sudo path)
- Replaced lingering `sudo systemctl ...` guidance with `systemctl --user` flow.

Files:
- `CLAUDE.md`
- `agents.md`

### 6) Runtime state cleanup / triage
- Corrected false-merged tasks (from quota-failure advancement bug) were already moved back to `failed`, then re-triaged.
- Task triage for range `84..106`:
  - keep failed (stale/invalid Zig-era): `84, 87, 88, 89, 107, 108`
  - keep merged (already integrated): `85, 86, 90, 91`
  - reset to backlog for fresh Rust-era reruns: `93..106`
- Additional reset to backlog for similarly affected later tasks: `109, 110, 111, 112, 114, 115`.
- Stuck branch refs for `task-107` and `task-108` removed.
- User/system borg service processes were stopped during cleanup to prevent concurrent DB mutation.

## Current key task states
- Active resumable tasks:
  - `#92` status=`impl` session present
  - `#113` status=`qa_fix` session present
- Backlog reset for rerun:
  - `#93..#106`, `#109`, `#110`, `#111`, `#112`, `#114`, `#115`

## Residual note
- Orphan directories `.worktrees/task-93..95` contain root-owned artifacts from prior sandbox/container runs and could not be fully deleted in this session due permissions.
- They are no longer registered git worktrees and no longer block pipeline operation.

## Verification run
- `cargo check` (workspace): pass
- `cargo test -p borg-core --lib`: pass (28/28)
