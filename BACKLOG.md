# Pending Work

Tasks that were completed by agents but not yet merged into main.
Cleared from integration queue on 2026-02-24.
Branch implementations still exist locally — can be cherry-picked or re-run.

---

## Bug Fixes

### Fix subprocess stdout/stderr sequential read deadlock
**Branches:** `feature/task-5`, `task-17` (duplicate)
In `git.zig`, `docker.zig`, `agent.zig`, `pipeline.zig`, `main.zig` — stdout is read to
completion before stderr. If a child process fills the OS pipe buffer (~64KB) writing to
stderr, the child blocks on write while the parent blocks on stdout read → deadlock.
Fix: read stdout and stderr concurrently (separate threads or poll).

### Fix OAuth token memory leak in refreshOAuthToken
**Branches:** `task-6`, `feature/task-20` (duplicate)
`config.zig:refreshOAuthToken()` overwrites `self.oauth_token` without freeing the previous
value. Called every 500ms in main loop → continuous memory growth.
Fix: free old token before assigning new one.

### Fix memory leak in pipeline tick() — task strings never freed
**Branch:** `task-18`
`pipeline.zig:tick()` allocates PipelineTask string fields per-tick but only frees the outer
slice, leaking ~10 strings per task every 30 seconds.
Fix: use arena allocator for the query, or free all task fields after thread completion.

### Fix WhatsApp stdout blocking the main event loop
**Branch:** `feature/task-4`
`whatsapp.zig:poll()` calls `stdout.read()` on a blocking pipe fd. When bridge has no data,
this blocks indefinitely, freezing Telegram polling, agent dispatch, cooldown expiry.
Fix: set O_NONBLOCK after spawn, or move read to dedicated thread.

### Fix Docker container name collision for concurrent agents
**Branch:** `task-8`
Container names use `std.time.timestamp()` (second granularity). Two agents spawned in the
same second get identical names — Docker rejects the second one.
Fix: add monotonic atomic counter or random suffix (already partially done in spawnAgent
but may not be complete).

### Consolidate duplicated child-process stdout/stderr drain pattern
**Branch:** `feature/task-3`
`git.zig`, `docker.zig`, `pipeline.zig`, `agent.zig` all copy-paste the same pattern:
read stdout/stderr into ArrayLists via 8192-byte buffer loop, extract exit code.
Fix: extract `drainPipe` helper and `exitCode` function into `process.zig`, replace copies.

### Consolidate getPipelineStats into a single SQL query
**Branch:** `task-14`
`db.zig:getPipelineStats` runs four separate COUNT queries. Replace with single query using
`COUNT(CASE WHEN ...)` expressions.

---

## Test Coverage

### json.zig — escapeString
**Branch:** `task-9`
Zero test coverage on `escapeString`. Need tests for `"`, `\`, `\n`, `\t`, `\r`,
control characters, empty string, normal ASCII, multi-byte UTF-8.

### json.zig — safe getter functions
**Branch:** `task-10`
`getString`, `getInt`, `getBool` untested. Need tests for missing key (returns default)
and present key (returns correct value).

### config.zig — getEnv parsing
**Branch:** `task-11`
`getEnv` parses `.env` file content but has no tests. Need tests for basic `KEY=VALUE`,
`#` comment lines, blank lines, values containing `=` (e.g. `TOKEN=abc=def`).

### config.zig — parseWatchedRepos
**Branch:** `task-12`
`parseWatchedRepos` splits `WATCHED_REPOS` env var but untested. Need tests for
pipe-delimited paths, single path, empty string.

### db.zig — registerGroup / getAllGroups round-trip
**Branch:** `task-13`
`registerGroup` and `getAllGroups` have zero test coverage. Need in-memory SQLite tests
verifying round-trip, field preservation, and `unregisterGroup` removal.

---

## Already Merged

- **task-15**: Guard against negative values in Telegram mention offset/length casts
  (merged in commit `c8a7b64`)
