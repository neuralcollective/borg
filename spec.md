TASK_START
TITLE: Fix WhatsApp stdout blocking the main event loop
DESCRIPTION: In whatsapp.zig:77-82, the `poll()` method calls `stdout.read()` on a blocking pipe fd. When the WhatsApp bridge has no data to send, this blocks indefinitely, freezing the entire main event loop (Telegram polling, agent dispatch, cooldown expiry). The pipe should be set to O_NONBLOCK after spawning the child process, or the read should be moved to a dedicated thread that feeds parsed events to the main loop via a thread-safe queue.
TASK_END

TASK_START
TITLE: Fix subprocess stdout/stderr sequential read deadlock
DESCRIPTION: In git.zig:27-40 and pipeline.zig:768-781, stdout is read to completion before stderr is read. If a child process fills the OS pipe buffer (~64KB on Linux) writing to stderr while also writing to stdout, the child blocks on stderr write, and the parent blocks on stdout read, causing a deadlock. Both streams must be drained concurrently, either by using separate threads per stream, poll/epoll, or by collecting both via the Zig child.collectOutput() pattern.
TASK_END

TASK_START
TITLE: Fix OAuth token memory leak in refreshOAuthToken
DESCRIPTION: In config.zig:106-110, `refreshOAuthToken` overwrites `self.oauth_token` with a newly allocated string without freeing the previous value. This function is called every main loop iteration (every 500ms in main.zig:647), causing continuous memory growth. The old token must be freed before assigning the new one, with care to avoid freeing the initial token that may have come from a non-owned slice.
TASK_END

TASK_START
TITLE: Fix use-after-free on pipeline shutdown with active agents
DESCRIPTION: In pipeline.zig:156, spawned `processTaskThread` thread handles are discarded (detached). During shutdown, `Pipeline.run()` waits at most 30s for agents then returns. The main function then destroys `pipeline_db` and the allocator, but detached agent threads may still be running and accessing `self.db` and `self.allocator`. Store thread handles and join them all during shutdown, or use a shared atomic flag that agents check before each db operation.
TASK_END

TASK_START
TITLE: Fix Docker container name collision for concurrent agents
DESCRIPTION: In pipeline.zig:1121-1123, container names are generated using `std.time.timestamp()` which has second granularity. If two pipeline agents are spawned within the same second, they get identical container names and Docker rejects the second container creation. Add a monotonic atomic counter or random suffix to ensure unique container names across concurrent spawns.
TASK_END

TASK_START
TITLE: Fix formatTimestamp panic on negative or zero timestamps
DESCRIPTION: In main.zig:1200, `@intCast(unix_ts)` casts i64 to u64 for EpochSeconds.secs. If a message has a negative or zero timestamp (e.g., timestamp 0 from WhatsApp defaults, or malformed Telegram data), this triggers a runtime panic from the safety-checked integer cast. Clamp the timestamp to a valid positive range (minimum 0) before casting, or use `@max(unix_ts, 0)` to prevent the panic.
TASK_END

TASK_START
TITLE: Fix data race on web_server_global pointer
DESCRIPTION: In main.zig:25, `web_server_global` is a plain `?*WebServer` read from the log function (called from any thread via std.log) and written from the main thread. This is a data race under the Zig/C memory model. Replace it with `std.atomic.Value(?*WebServer)` and use atomic load/store with appropriate memory ordering to ensure correct visibility across threads.
TASK_END

TASK_START
TITLE: Fix memory leak of session_id in /tasks command handler
DESCRIPTION: In main.zig:1089-1103, the `/tasks` command handler frees individual fields of each PipelineTask struct (title, description, repo_path, branch, status, last_error, created_by, notify_chat, created_at) but omits `t.session_id`. Since `rowToPipelineTask` allocates `session_id` via `allocator.dupe`, this leaks memory on every `/tasks` invocation. Add `allocator.free(t.session_id)` to the cleanup block.
TASK_END

TASK_START
TITLE: Fix JSON injection via unescaped JID in WhatsApp sendMessage
DESCRIPTION: In whatsapp.zig:161, the `jid` parameter is interpolated directly into a JSON string without escaping. A JID containing `"` or `\` would produce malformed JSON sent to the bridge process stdin, potentially causing message send failures or protocol desync. Apply `json_mod.escapeString` to the JID before interpolation, consistent with the escaping already done for the text field.
TASK_END

TASK_START
TITLE: Fix Telegram entity offset/length panic on negative values
DESCRIPTION: In telegram.zig:108-109, `@intCast` converts i64 offset/length values from the Telegram API to usize. A malformed or adversarial API response with negative offset or length values will cause a runtime panic crashing the entire process. Replace `@intCast` with `std.math.cast(usize, ...)` which returns null on out-of-range values, and skip the entity on failure via `orelse continue`.
TASK_END

TASK_START
TITLE: Fix web server single-read HTTP request parsing
DESCRIPTION: In web.zig:137-141, `handleConnection` performs a single `stream.read()` to get the HTTP request. TCP may deliver the request across multiple segments, causing partial reads where only part of the request line is received. The path parsing then operates on incomplete data, leading to incorrect routing or dropped requests under load. Read in a loop until `\r\n` (end of request line) is found, with a timeout and max-size limit to prevent slow-loris attacks.
TASK_END

TASK_START
TITLE: Add size limit to subprocess stdout/stderr buffers
DESCRIPTION: In git.zig, agent.zig, docker.zig, and pipeline.zig, subprocess stdout/stderr is read into unbounded ArrayLists with no size cap. A misbehaving subprocess could write gigabytes of output, causing OOM and crashing the entire orchestrator. Add a configurable maximum buffer size (e.g., 50MB for agent output, 10MB for git/test output) and stop reading once exceeded, truncating the output.
TASK_END

TASK_START
TITLE: Fix isBindSafe Docker mount check to resolve symlinks
DESCRIPTION: In docker.zig:253-279, `isBindSafe` checks the string representation of the host path against a blocklist of sensitive directories. A symlink (e.g., `/tmp/innocent` pointing to `/home/user/.ssh`) bypasses all checks because only the literal string is examined. Use `std.fs.realpathAlloc` to resolve the actual filesystem path before checking against the blocklist, preventing symlink-based mount escapes in pipeline agent containers.
TASK_END

TASK_START
TITLE: Fix SSE client list resource leak for idle connections
DESCRIPTION: In web.zig:261, SSE clients are added to `sse_clients` and only removed when a write fails during `broadcastSse`. During quiet periods with no log events, disconnected clients accumulate as stale entries holding open file descriptors. Add a periodic keepalive mechanism (e.g., send SSE comment `: keepalive\n\n` every 30 seconds) that also serves to detect and prune dead connections via write failure.
TASK_END

TASK_START
TITLE: Fix deadlock risk from nested mutex acquisition in pushLog
DESCRIPTION: In web.zig:55-75, `pushLog` acquires `log_mu` then calls `broadcastSse` which acquires `sse_mu`, establishing an implicit lock ordering (log_mu then sse_mu). While no current code path reverses this order, it is fragile and undocumented. Refactor to release `log_mu` before calling `broadcastSse` by copying the level and message into local buffers first, eliminating the nested lock acquisition entirely.
TASK_END
