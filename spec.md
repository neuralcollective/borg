TASK_START
TITLE: Add tests for parseWatchedRepos in config.zig
DESCRIPTION: parseWatchedRepos (config.zig:118) has no tests despite handling complex pipe-delimited parsing with colon-separated path:cmd pairs, duplicate-primary skipping, empty entry filtering, and default test command fallback. Add tests for: multiple repos with pipe delimiter, entries without colon (default cmd), duplicate primary repo skipping, empty/whitespace-only entries, and entries with leading/trailing whitespace around paths and commands.
TASK_END

TASK_START
TITLE: Add tests for getTestCmdForRepo in config.zig
DESCRIPTION: Config.getTestCmdForRepo (config.zig:103) has no test coverage. It iterates watched_repos looking for a matching path and falls back to pipeline_test_cmd. Add tests for: exact match returns the repo-specific command, no match returns the default pipeline_test_cmd, and behavior with an empty watched_repos list.
TASK_END

TASK_START
TITLE: Add tests for json.stringify in json.zig
DESCRIPTION: json.stringify (json.zig:83) has zero test coverage. It converts a std.json.Value back to a JSON string, which is used for serialization throughout the codebase. Add tests for: stringifying an object with mixed types, stringifying a simple string value, stringifying null, and stringifying nested objects and arrays to verify round-trip correctness with parse.
TASK_END

TASK_START
TITLE: Add tests for json accessor functions with non-object input
DESCRIPTION: getInt, getBool, getObject, and getArray (json.zig:23-58) all check `if (obj != .object) return null` but only getString has a test for wrong-type input. Add tests verifying that all five accessor functions return null when passed a non-object Value (e.g., a string or array Value), and that getInt correctly handles the float-to-int coercion path (json.zig:28).
TASK_END

TASK_START
TITLE: Add tests for decodeChunked edge cases in http.zig
DESCRIPTION: decodeChunked (http.zig:155) only has two happy-path tests. Add tests for: empty input (no chunks), chunk with size 0 immediately (empty body), malformed hex chunk size (non-hex characters should break cleanly), truncated chunk data (pos + chunk_size > data.len), and missing terminal \r\n between chunks. These edge cases arise from Docker API responses over Unix sockets.
TASK_END

TASK_START
TITLE: Add tests for web.zig parseMethod and parseBody functions
DESCRIPTION: web.zig has a test for parsePath but none for parseMethod (line 232) or parseBody (line 249). Add tests for: parseMethod with GET/POST/DELETE requests and malformed input (no space), parseBody extracting content after \r\n\r\n separator, parseBody returning empty string when no body separator exists, and parseBody with an empty body after headers.
TASK_END

TASK_START
TITLE: Add tests for web.zig guessContentType function
DESCRIPTION: guessContentType (web.zig:728) maps file extensions to MIME types for static file serving but has no tests. Add tests verifying correct MIME types for .html, .css, .js, .json, .svg, .png, .ico extensions, and that unknown extensions return "application/octet-stream". This function affects cache headers and browser behavior.
TASK_END

TASK_START
TITLE: Add tests for web.zig jsonEscapeAlloc with truncation
DESCRIPTION: jsonEscapeAlloc (web.zig:739) handles heap-allocated JSON escaping with a 16000-char truncation limit, used for task output display. The existing jsonEscape tests only cover the stack-buffer variant. Add tests for: input under the limit, input exactly at the 16000-char limit, input exceeding the limit (verify "... (truncated)" suffix), and control character stripping (chars < 0x20 are dropped, unlike json.zig escapeString which encodes them as \u00XX).
TASK_END

TASK_START
TITLE: Add tests for isBindSafe edge cases in docker.zig
DESCRIPTION: isBindSafe (docker.zig:253) has basic tests but misses important edge cases. Add tests for: empty string input, bind with no colon (already tested but verify returns false), path ending with a blocked suffix (e.g., "/home/.sshkeys" should pass since ".ssh" is a substring match), root-level sensitive paths ("/root/.ssh:/x"), and paths containing the blocked pattern mid-path vs as a directory name (e.g., "/data/.env.backup:/x" should be blocked since ".env" substring matches).
TASK_END

TASK_START
TITLE: Add tests for agent.zig parseNdjson with system-type session_id
DESCRIPTION: parseNdjson (agent.zig:11) handles session_id from both "result" and "system" type messages, but only "result" type is tested. Add a test where session_id comes from a "system" init message before any result, verifying the session_id is captured. Also test: session_id from "system" being overwritten by a later "result" session_id, and data containing only "system" messages with no "result" (output should be empty string).
TASK_END

TASK_START
TITLE: Add tests for WhatsApp parseEvent NDJSON parsing
DESCRIPTION: WhatsApp.parseEvent (whatsapp.zig:119) parses five event types from the bridge (message, connected, qr, disconnected, error) but has no tests beyond init/deinit. Add unit tests that call parseEvent directly with JSON strings for each event type, verifying correct field extraction. Also test: malformed JSON returns null, missing "event" field returns null, unknown event type returns null, and message with missing optional fields uses defaults.
TASK_END

TASK_START
TITLE: Add tests for Discord parseEvent NDJSON parsing
DESCRIPTION: Discord.parseEvent (discord.zig:124) parses three event types (message, ready, error) from the bridge but has no tests beyond init/deinit. Add unit tests calling parseEvent with JSON strings for each event type. Verify: message fields (channel_id, sender_name, is_dm, mentions_bot) are correctly extracted, ready event captures bot_id, error event captures message, malformed JSON returns null, and unknown event types return null.
TASK_END

TASK_START
TITLE: Add tests for Telegram sendMessage chunking logic
DESCRIPTION: Telegram.sendMessage (telegram.zig:149) chunks messages at 4000-char boundaries and only applies reply_to on the first chunk. This chunking logic is testable in isolation by refactoring or by verifying the loop behavior. Add tests for: message under 4000 chars (single chunk), message exactly 4000 chars, message of 8001 chars (three chunks), and empty message. Focus on verifying the offset arithmetic and chunk boundary correctness.
TASK_END

TASK_START
TITLE: Add tests for db.zig task output storage and retrieval
DESCRIPTION: storeTaskOutput (db.zig:613) and getTaskOutputs (db.zig:629) have no direct tests despite being used in the pipeline and web dashboard. Add tests for: storing and retrieving outputs for a task, storing multiple outputs for the same task (verifying ordering by created_at), output truncation at 32000 chars (storeTaskOutput:614), and retrieving outputs for a task with no outputs (empty slice).
TASK_END

TASK_START
TITLE: Add tests for db.zig pipeline task priority ordering
DESCRIPTION: getNextPipelineTask (db.zig:363) uses a CASE statement to prioritize task statuses: rebase > retry > impl > qa > spec > backlog. This ordering is critical for pipeline correctness but untested. Add a test that creates tasks with different statuses and verifies getNextPipelineTask returns them in priority order, not creation order. Also verify that tasks with status 'done', 'merged', or 'failed' are excluded.
TASK_END

TASK_START
TITLE: Add tests for db.zig resetTaskAttempt clears branch and session
DESCRIPTION: resetTaskAttempt (db.zig:464) resets attempt to 0 and clears branch and session_id, which is critical for task retry behavior. Add a test that creates a task, sets its branch/session_id/attempt, calls resetTaskAttempt, then verifies all three fields are cleared. Currently only incrementTaskAttempt is tested in the pipeline task lifecycle test.
TASK_END

TASK_START
TITLE: Add tests for db.zig getPipelineStats counts
DESCRIPTION: getPipelineStats (db.zig:511) runs four separate COUNT queries categorizing tasks by status. It is used by the dashboard status endpoint but has no test. Add a test that creates tasks in various statuses (backlog, spec, impl, merged, failed) and verifies the stats counts are correct: active includes all non-terminal statuses, merged and failed are distinct, and total is the sum of all.
TASK_END

TASK_START
TITLE: Add tests for db.zig getQueuedBranchesForRepo filtering
DESCRIPTION: getQueuedBranchesForRepo (db.zig:575) filters the integration queue by repo_path, used for multi-repo pipeline support. It has no tests. Add tests verifying: entries for the target repo are returned, entries for other repos are excluded, and an empty result when no entries match. Also verify resetStuckQueueEntries (db.zig:597) changes 'merging' entries back to 'queued'.
TASK_END

TASK_START
TITLE: Add tests for sqlite.zig Row.get and Row.getInt bounds checking
DESCRIPTION: sqlite.Row.get (sqlite.zig:31) returns null for out-of-bounds column indices, and Row.getInt (sqlite.zig:36) parses text to i64 with a catch-null fallback. These are used in every db.zig query but only tested indirectly. Add direct unit tests for: get with valid index, get with out-of-bounds index, getInt with valid integer text, getInt with non-numeric text (returns null), and getInt with empty string (returns null).
TASK_END

TASK_START
TITLE: Add tests for web.zig pushLog ring buffer wraparound
DESCRIPTION: WebServer.pushLog (web.zig:73) writes to a 500-entry ring buffer with head/count tracking. The wraparound logic (log_head modulo LOG_RING_SIZE, count capped at LOG_RING_SIZE) is untested. Add a test that pushes more than 500 log entries and verifies: log_count never exceeds 500, log_head wraps correctly, and the oldest entries are overwritten while recent entries are preserved.
TASK_END
