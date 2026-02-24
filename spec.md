# Spec: Add tests for `parseWatchedRepos` in config.zig

## Task Summary

The `parseWatchedRepos` function in `src/config.zig` (lines 121-160) parses the `WATCHED_REPOS` environment variable into a slice of `RepoConfig` structs but has no test coverage. Add tests for pipe-delimited multi-repo input, single-path input, and empty/missing input to prevent regressions where the pipeline watches wrong or no repositories.

## Files to Modify

1. **`src/config.zig`** — Add new `test` blocks at the end of the file (after line 270), following the existing pattern used by `findEnvValue` tests.

## Files to Create

None. Tests belong inline in `src/config.zig` since `parseWatchedRepos` is a private (non-`pub`) function and can only be called from within the same file.

## Function/Type Signatures

No new functions or types. The tests call the existing private function directly:

```zig
fn parseWatchedRepos(allocator: std.mem.Allocator, env_content: []const u8, primary_repo: []const u8, primary_test_cmd: []const u8) ![]RepoConfig
```

Each test block follows this pattern:

```zig
test "parseWatchedRepos <description>" {
    const alloc = std.testing.allocator;
    // Construct env_content with WATCHED_REPOS=... as the function reads it via getEnv()
    const env = "WATCHED_REPOS=<value>";
    const repos = try parseWatchedRepos(alloc, env, "<primary_repo>", "<primary_test_cmd>");
    defer alloc.free(repos);
    // For entries with duped strings, also free them:
    defer for (repos) |r| {
        if (!r.is_self) {
            alloc.free(r.path);
            // Only free test_cmd if it was duped (not the default "make test" literal)
        }
    };
    // assertions...
}
```

## Acceptance Criteria

1. **Empty WATCHED_REPOS, no primary repo**: Calling `parseWatchedRepos(alloc, "", "", "")` returns a zero-length slice (`repos.len == 0`).

2. **Empty WATCHED_REPOS, with primary repo**: Calling with `env_content=""`, `primary_repo="/home/project"`, `primary_test_cmd="zig build test"` returns exactly one entry where `repos[0].path` equals `"/home/project"`, `repos[0].test_cmd` equals `"zig build test"`, and `repos[0].is_self == true`.

3. **Single path without test command**: `WATCHED_REPOS=/repo/a` (no colon) returns the primary repo plus one additional entry with `.path == "/repo/a"`, `.test_cmd == "make test"` (the default), and `.is_self == false`.

4. **Single path with test command**: `WATCHED_REPOS=/repo/a:npm test` returns the primary repo plus one entry with `.path == "/repo/a"` and `.test_cmd == "npm test"`.

5. **Pipe-delimited multiple repos**: `WATCHED_REPOS=/repo/a:cmd1|/repo/b:cmd2` returns the primary repo plus two additional entries with correct paths and commands, in order.

6. **Duplicate of primary repo is skipped**: If `primary_repo="/main"` and `WATCHED_REPOS=/main:other_cmd|/second:test`, the result contains only the primary entry and `/second` — the `/main` duplicate from WATCHED_REPOS is excluded.

7. **Whitespace trimming**: Entries with surrounding spaces/tabs (e.g., `WATCHED_REPOS= /repo/a : cmd1 | /repo/b `) are trimmed correctly, producing entries with clean paths and commands.

8. **Empty entries between pipes are skipped**: `WATCHED_REPOS=/repo/a||/repo/b` skips the empty middle entry and returns two watched repos (plus primary if set).

9. **Build succeeds**: `zig build` compiles without errors.

10. **All tests pass**: `zig build test` passes, including all new and existing tests.

## Edge Cases

1. **Completely empty env_content and no primary repo** — should return `&.{}` (empty slice, length 0), not an error.

2. **WATCHED_REPOS is only whitespace/pipes** (e.g., `WATCHED_REPOS=| | |`) — all entries trim to empty and are skipped, so result contains only the primary repo (if set) or is empty.

3. **Entry with colon but empty path** (e.g., `WATCHED_REPOS=:some_cmd`) — the path is empty after trim, so the entry is skipped (line 141: `if (path.len == 0) continue`).

4. **Entry with colon but empty command** (e.g., `WATCHED_REPOS=/repo/a:`) — the command is empty, so the default `"make test"` is used (line 145: `if (cmd.len > 0) ... else "make test"`).

5. **Path without colon matching primary repo** — entry is skipped (line 149: `if (std.mem.eql(u8, trimmed, primary_repo)) continue`).

6. **Memory cleanup in tests** — tests must free the returned slice via `alloc.free(repos)` and free any allocator-duped strings (`.path` and `.test_cmd` on non-`is_self` entries where `test_cmd` was duped) to satisfy `std.testing.allocator` leak detection.
