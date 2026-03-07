# Borg Security & Code Quality Audit

**Date:** 2026-03-07
**Scope:** Full repo — borg-server, borg-core, borg-agent, borg-domains, dashboard, deploy, sidecar, container

---

## CRITICAL

### C1. Unauthenticated API token exposure
`borg-server/src/auth.rs:117,169-171`
`/api/auth/token` is auth-exempt and returns the shared admin-equivalent API token to any caller. Anyone who can reach the server gets full admin access. The entire auth system (JWT, passwords, argon2) is bypassed.

### C2. XSS via dangerouslySetInnerHTML on search snippets
`dashboard/src/components/projects-panel.tsx:755-756`
`title_snippet` and `content_snippet` from search results are rendered with `dangerouslySetInnerHTML`. Uploaded documents containing HTML/script tags execute in the user's browser — stored XSS.

### C3. YQL injection in Vespa search_chunks
`borg-server/src/vespa.rs:437,440`
`doc_type` and `jurisdiction` filter values from query params are interpolated directly into YQL without escaping. A value like `' or true or doc_type contains '` breaks out of the filter.

### C4. Docker signal marker prefix mismatch — agent signals silently lost
`container/entrypoint.sh:203` emits `---BORG_SIGNAL---{json}`
`borg-agent/src/claude.rs:16` parses `BORG_SIGNAL:`
These never match. Additionally, the signal file path differs (work_dir/.borg vs work_dir/repo/.borg). Blocked/abandon signals in Docker mode are silently lost — blocked agents get retried instead of paused.

### C5. signal_json swapped into new_session_id — corrupts session resumption
`borg-agent/src/claude.rs:525-533`
Signal JSON content is assigned to `PhaseOutput.new_session_id` instead of `signal_json`. The pipeline stores it as the task's session_id. Next run passes garbage as `--resume {"status":"blocked"}`.

### C6. Missing kill_on_drop in Codex/Gemini backends — orphan subprocesses
`borg-agent/src/codex.rs:126-148`, `borg-agent/src/gemini.rs:80-92`
Subprocesses spawned without `.kill_on_drop(true)`. If the tokio task is cancelled, `codex exec --full-auto` or `gemini --approval-mode=yolo` continues running unsupervised.

### C7. Proxy routes served on main port without auth
`borg-server/src/main.rs:813-814`
The proxy router (`/v1/messages`, `/v1/search`) is merged before auth middleware and paths don't start with `/api/`, so they're auth-exempt. Anyone reaching port 3131 can invoke Bedrock LLM calls and web searches without authentication.

---

## HIGH

### H1. Claude timeout doesn't kill bwrap child process
`borg-agent/src/claude.rs:498-504`
On timeout, the code returns empty output but never calls `child.kill()`. In bwrap mode, `kill_on_drop` sends SIGKILL to bwrap but may not propagate to the Claude subprocess inside.

### H2. Full host environment inherited by all agent subprocesses
`borg-agent/src/claude.rs:258-283,365-392`, `codex.rs:126-148`, `gemini.rs:80-92`
No mode calls `.env_clear()`. Subprocesses inherit `ANTHROPIC_API_KEY`, `AWS_SECRET_ACCESS_KEY`, database credentials, etc.

### H3. Bwrap --unshare-all blocks network — agents can't reach Anthropic API
`borg-agent/src/sandbox.rs:125-134`
`--unshare-all` unshares the network namespace with no `--share-net`. Bwrap-sandboxed agents have no network access.

### H4. Shared /tmp in bwrap breaks isolation
`borg-agent/src/sandbox.rs:123`
`/tmp` is bind-mounted RW from host. Concurrent sandboxed tasks share `/tmp`, enabling cross-task data leakage.

### H5. Unbounded output_lines accumulation — potential OOM
`borg-agent/src/claude.rs:431`, `codex.rs:155`, `gemini.rs:99`
`Vec<String>` grows without cap. An agent producing continuous output exhausts host memory.

### H6. get_settings exposes secrets to non-admin users
`borg-server/src/routes.rs:3996-4035`
No admin check (unlike `put_settings`). Any authenticated user reads OAuth client secrets, S3 credentials, Vespa URLs. Combined with C1, all secrets are public.

### H7. No concurrency limit on chat agent spawning
`borg-server/src/routes.rs:4519-4537`
Rate limit is per-thread-name but attackers can create unlimited thread names. Each request spawns a Claude process with no semaphore.

### H8. Command injection in container web_search shim
`container/entrypoint.sh:75-78`
`$QUERY` interpolated directly into JSON via shell expansion. Queries containing `"`, `$(cmd)`, or backticks break out. Fix: use `jq` for JSON construction.

### H9. SET TIME ZONE 'UTC' on every DB call
`borg-core/src/pgcompat.rs:276`
`guard()` runs `SET TIME ZONE 'UTC'` on every single DB method call, doubling round-trips. Should be set once per connection.

### H10. search_embeddings full table scan with in-memory cosine similarity
`borg-core/src/db.rs:2399-2463`
Loads all embedding rows into memory and computes similarity in Rust. Should use pgvector.

### H11. GitHub token embedded in git remote URL
`borg-core/src/pipeline.rs:1873`
Token in `https://x-access-token:{token}@github.com/...` is stored in `.git/config`, visible in error messages and process listings. Should use GIT_ASKPASS.

### H12. expect() panics in production paths
`borg-core/src/pgcompat.rs:534-535` — Runtime::new().expect()
`borg-core/src/knowledge.rs:78` — reqwest Client build .expect()
Both crash the entire process instead of propagating errors.

### H13. default_max_attempts is dead code — always hardcoded to 5
`borg-core/src/types.rs:291` defines per-mode defaults (e.g., lawborg=3).
All task creation sites hardcode `max_attempts: 5`. Lawborg gets 5 retries instead of intended 3.

### H14. pipeline_agent_cooldown_s not loaded from DB
Dashboard writes it to DB but `Config::load_from_db()` never reads it. Dashboard changes are silently ignored.

### H15. Duplicate derive_compile_check with different logic
`borg-agent/src/claude.rs:34` uses `starts_with("cargo test")`
`borg-core/src/pipeline.rs:37` uses `contains("cargo test")`
`cd src && cargo test` handled differently between Docker and non-Docker.

### H16. Nine dead legal API client modules
`borg-domains/src/legal/{clio,cognitive,edgar,federal_register,intelligize,lexis,lexmachina,statenet,westlaw}.rs`
Exported but never used. Add compile time and dependency surface. Should be feature-gated.

---

## MEDIUM

### M1. Blocking file I/O on async runtime
`borg-server/src/routes.rs:4716,4772,4848,2517,769-770`
`std::fs` operations (create_dir_all, write, read) in async handlers without spawn_blocking.

### M2. No rate limiting on login endpoint
`borg-server/src/auth.rs:248-282`
Combined with 4-char minimum password, brute-force is practical.

### M3. TOCTOU race in setup endpoint
`borg-server/src/auth.rs:191-239`
Concurrent requests to `/api/auth/setup` could both see zero users and create two admin accounts.

### M4. Unbounded cloud file download
`borg-server/src/routes.rs:5614-5691`
Cloud provider files downloaded fully into memory without size limits.

### M5. chat_rate HashMap grows unbounded
`borg-server/src/routes.rs:4474`
Entries never removed. Unique thread names cause unbounded memory growth.

### M6. JWT 30-day expiry with no revocation
`borg-server/src/auth.rs:36-53`
No way to revoke individual tokens between server restarts.

### M7. std::process::exit(0) in async runtime
`borg-core/src/pipeline.rs:903`
Terminates without running destructors, flushing buffers, or completing in-flight tasks. Should signal graceful shutdown.

### M8. Synchronous git commands blocking tokio threads
`borg-core/src/pipeline.rs:2364,2427,2449`, `knowledge.rs:236,252`
`std::process::Command` in async context blocks worker threads.

### M9. web_search vs WebSearch tool name mismatch
Legal mode uses `"web_search"` (correct). All other modes use `"WebSearch"` (incorrect). One set has broken tool references.

### M10. Missing error_instruction on sales/crew/chef implement phases
Retry agents in these modes don't see previous error output.

### M11. resolve_mode silently falls back to sweborg
`borg-core/src/pipeline.rs:131-142`
Unknown mode name → sweborg. A lawborg task with a typo runs through the SWE pipeline.

### M12. PhaseOutput.signal_json field is completely dead
Set to `None` in every backend, never read by pipeline. Pipeline reads signals from disk instead.

### M13. model_override setting is half-wired
Appears in settings and status but never applied to Config.model. Dashboard changes have no effect.

### M14. Hardcoded credentials in docker-compose
`deploy/docker-compose.stack.yml:8,47` — `borg`, `seaweedfssecret`
Ports bound to 0.0.0.0. Docker iptables may bypass ufw.

### M15. SSH CIDR default is 0.0.0.0/0
`deploy/terraform/hybrid/variables.tf:63-67`
Should default to empty, forcing deployer to specify allowed ranges.

---

## LOW

### L1. Dead code: verify_token in auth.rs:124
### L2. Duplicate sha256_hex_file variants and extract_text_from_bytes copies
### L3. CLAUDE.md written to session_dir without symlink check
### L4. CID file in world-writable /tmp — TOCTOU race
### L5. raw_stream.clone() doubles memory for output
### L6. Minimum password length of 4 characters
### L7. create_jwt silently returns empty string on failure
### L8. detect_doc_type operator precedence bug (ingestion.rs:269,275)
### L9. URL path segments not encoded in dead legal API clients
### L10. Missing include_file_listing on sales implement phase
### L11. ComplianceCheck phase type defined but no mode uses it
### L12. pending_review status outside is_terminal()
### L13. Hardcoded proxy listener on port 3132
### L14. git add -A in container commits potential secrets
