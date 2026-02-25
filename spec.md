# Spec: Add tests for Config.getTestCmdForRepo

## Task Summary

`Config.getTestCmdForRepo` (`src/config.zig:106`) iterates `self.watched_repos` looking for a `RepoConfig` whose `path` exactly matches the given `repo_path`, returning its `test_cmd` on match or falling back to `self.pipeline_test_cmd`. The function has no test coverage. Three test cases must be added: exact match, no match (fallback), and empty `watched_repos`.

## Files to Modify

1. **`src/config.zig`** — Append three tests to the existing `// ── Tests ──` section at the bottom of the file (after the last `findEnvValue` test, currently ending at line 270).

## Files to Create

None.

## Function/Type Signatures for New or Changed Code

No signatures change. The existing function under test is:

```zig
// src/config.zig:106
pub fn getTestCmdForRepo(self: *Config, repo_path: []const u8) []const u8
```

The tests construct a stack-allocated `Config` value with only the two fields that `getTestCmdForRepo` reads — `watched_repos` and `pipeline_test_cmd` — set to meaningful values. All other `Config` fields are set to zero/empty defaults to satisfy the struct literal. A private helper `testMinimalConfig` is added above the tests to avoid repeating the full struct literal in each test:

```zig
// Private helper — only used in tests, defined above the test blocks.
fn testMinimalConfig(pipeline_test_cmd: []const u8, watched_repos: []RepoConfig) Config {
    return Config{
        .telegram_token = "",
        .oauth_token = "",
        .assistant_name = "",
        .trigger_pattern = "",
        .data_dir = "",
        .container_image = "",
        .model = "",
        .credentials_path = "",
        .session_max_age_hours = 0,
        .max_consecutive_errors = 0,
        .pipeline_repo = "",
        .pipeline_test_cmd = pipeline_test_cmd,
        .pipeline_lint_cmd = "",
        .pipeline_admin_chat = "",
        .release_interval_mins = 0,
        .continuous_mode = false,
        .collection_window_ms = 0,
        .cooldown_ms = 0,
        .agent_timeout_s = 0,
        .max_concurrent_agents = 0,
        .rate_limit_per_minute = 0,
        .web_port = 0,
        .dashboard_dist_dir = "",
        .watched_repos = watched_repos,
        .whatsapp_enabled = false,
        .whatsapp_auth_dir = "",
        .discord_enabled = false,
        .discord_token = "",
        .graphite_enabled = false,
        .allocator = std.testing.allocator,
    };
}
```

The three new test functions (no signatures beyond the `test "..."` blocks):

```zig
test "getTestCmdForRepo exact match returns repo-specific command"
test "getTestCmdForRepo no match returns pipeline_test_cmd default"
test "getTestCmdForRepo empty watched_repos returns pipeline_test_cmd default"
```

## Acceptance Criteria

1. **Exact match**: Given a `Config` with `pipeline_test_cmd = "zig build test"` and `watched_repos = &.{.{ .path = "/repos/myapp", .test_cmd = "npm test", .is_self = false }}`, calling `config.getTestCmdForRepo("/repos/myapp")` returns `"npm test"`.

2. **No match — fallback**: Given the same `Config` above, calling `config.getTestCmdForRepo("/repos/other")` returns `"zig build test"` (the `pipeline_test_cmd` default).

3. **Empty `watched_repos` — fallback**: Given a `Config` with `pipeline_test_cmd = "make test"` and `watched_repos = &.{}`, calling `config.getTestCmdForRepo("/any/path")` returns `"make test"`.

4. **`zig build test` passes**: All three new tests and all pre-existing tests in `src/config.zig` pass without modification to any other file.

5. **No allocations in tests**: The tests use only stack/comptime-literal data (slice literals via `&.{...}`); no heap allocation or `defer` is required for these three tests.

## Edge Cases to Handle

1. **Path prefix must not match**: A `watched_repos` entry with `path = "/repos/app"` must not match a query for `"/repos/application"`. The comparison uses `std.mem.eql` (byte-exact), so this is already correct — the test for "no match" covers this implicitly by querying a different path.

2. **Multiple repos — first match wins**: If `watched_repos` contains two entries with distinct paths, only the entry whose `path` equals `repo_path` is returned. The function returns on the first match, so order matters; the test for exact match should include at least one non-matching entry before the matching one to verify the loop does not short-circuit incorrectly.

3. **`pipeline_test_cmd` is empty string**: If `pipeline_test_cmd = ""` and there is no match, `getTestCmdForRepo` returns `""`. No special handling is needed; returning an empty string is correct behaviour (the caller decides what to do with it).

4. **`watched_repos` contains only entries whose paths do not match**: The fallback to `pipeline_test_cmd` must occur even when `watched_repos` is non-empty. The "no match" test covers this case.
