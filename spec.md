# Spec: Add tests for `parseWatchedRepos` in config.zig

## 1. Task Summary

`parseWatchedRepos` in `src/config.zig:118-157` parses the `WATCHED_REPOS` environment variable into a slice of `RepoConfig` structs but has no test coverage. Tests must be added for pipe-delimited multiple paths, single-path input, path-with-test-command input, and empty string input (should return empty slice when no primary repo is set). Note: the actual delimiter is pipe (`|`), not comma; the task description's "comma-separated" is interpreted as "delimited list."

## 2. Files to Modify

- `src/config.zig` — Add test blocks at the end of the file (after line 267), following the existing pattern used by the `findEnvValue` tests already present in the file.

## 3. Files to Create

None. Tests go directly in `src/config.zig` since `parseWatchedRepos` is a private (`fn`, not `pub fn`) function and can only be tested from within the same file. This follows the existing convention in the file (see `findEnvValue` tests at lines 208-267). The build system already includes these tests via `zig build test` (build.zig:54-59 creates a test from the `exe_mod` root module `src/main.zig`, which imports `config.zig`).

## 4. Function/Type Signatures

No new public functions or types are needed. The tests call the existing private function directly:

```zig
fn parseWatchedRepos(
    allocator: std.mem.Allocator,
    env_content: []const u8,
    primary_repo: []const u8,
    primary_test_cmd: []const u8,
) ![]RepoConfig
```

Where `RepoConfig` is (defined at `src/config.zig:3-7`):

```zig
pub const RepoConfig = struct {
    path: []const u8,
    test_cmd: []const u8,
    is_self: bool,
};
```

Each test block should use `std.testing.allocator` (or an `ArenaAllocator` wrapping it) and craft an `env_content` string containing `WATCHED_REPOS=...` to control the parsed value. The function internally calls `getEnv(allocator, env_content, "WATCHED_REPOS")`, so providing the key in `env_content` is sufficient.

**Important allocator note:** `parseWatchedRepos` allocates duped strings for secondary repo paths and test commands via `allocator.dupe`. Tests using `std.testing.allocator` must free the returned slice and its contents to avoid leak detection failures. Use an `ArenaAllocator` to simplify cleanup, consistent with the pattern in `src/is_bot_message_test.zig`.

## 5. Acceptance Criteria

### AC1: Empty WATCHED_REPOS with no primary repo returns empty slice
Call `parseWatchedRepos(allocator, "", "", "")`. Assert the returned slice has length 0.

### AC2: Empty WATCHED_REPOS with a primary repo returns slice of length 1
Call `parseWatchedRepos(allocator, "", "/repo/primary", "zig build test")`. Assert:
- Returned slice has length 1
- `repos[0].path` equals `"/repo/primary"`
- `repos[0].test_cmd` equals `"zig build test"`
- `repos[0].is_self` is `true`

### AC3: Single path in WATCHED_REPOS (no test command)
Craft `env_content = "WATCHED_REPOS=/repo/secondary"` and call with empty primary repo. Assert:
- Returned slice has length 1
- `repos[0].path` equals `"/repo/secondary"`
- `repos[0].test_cmd` equals `"make test"` (the default)
- `repos[0].is_self` is `false`

### AC4: Single path with test command in WATCHED_REPOS
Craft `env_content = "WATCHED_REPOS=/repo/secondary:cargo test"`. Assert:
- `repos[0].path` equals `"/repo/secondary"`
- `repos[0].test_cmd` equals `"cargo test"`
- `repos[0].is_self` is `false`

### AC5: Pipe-delimited multiple paths in WATCHED_REPOS
Craft `env_content = "WATCHED_REPOS=/repo/a:/repo/a/cmd|/repo/b"` with a primary repo set. Assert:
- Returned slice has length 3 (primary + 2 watched)
- `repos[0]` is the primary with `is_self = true`
- `repos[1].path` equals `"/repo/a"` with `test_cmd` `"/repo/a/cmd"` and `is_self = false`
- `repos[2].path` equals `"/repo/b"` with `test_cmd` `"make test"` and `is_self = false`

### AC6: WATCHED_REPOS entry matching primary repo is skipped
Craft `env_content = "WATCHED_REPOS=/repo/primary|/repo/other"` and call with `primary_repo = "/repo/primary"`. Assert:
- Returned slice has length 2 (primary + `/repo/other` only)
- No entry with `is_self = false` has path equal to `"/repo/primary"`

### AC7: All tests pass with `zig build test`
Running `zig build test` succeeds with zero failures.

## 6. Edge Cases to Handle

### E1: Whitespace-only entries are skipped
`env_content = "WATCHED_REPOS=/repo/a|  |/repo/b"` — the whitespace-only middle entry is ignored; result contains entries for `/repo/a` and `/repo/b` only.

### E2: Whitespace around paths is trimmed
`env_content = "WATCHED_REPOS=  /repo/a : my test cmd  |  /repo/b  "` — paths and test commands have leading/trailing whitespace trimmed.

### E3: Entry with colon but empty path is skipped
`env_content = "WATCHED_REPOS=:some_cmd"` — the path portion is empty after splitting on `:`, so the entry is skipped (line 138: `if (path.len == 0) continue`).

### E4: Entry with colon but empty test command gets default
`env_content = "WATCHED_REPOS=/repo/a:"` — the test command portion is empty, so it defaults to `"make test"` (line 142: `if (cmd.len > 0) ... else "make test"`).

### E5: Trailing pipe produces no extra entry
`env_content = "WATCHED_REPOS=/repo/a|"` — the trailing empty segment is skipped.

### E6: Primary repo with empty path does not add a self entry
Call with `primary_repo = ""` — no `is_self = true` entry is added (line 122: `if (primary_repo.len > 0)`).
