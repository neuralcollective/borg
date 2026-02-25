# Spec: Add tests for parseWatchedRepos in config.zig

## Task Summary

The `parseWatchedRepos` function (`src/config.zig:121`) parses pipe-delimited `WATCHED_REPOS` entries with colon-separated `path:cmd` pairs, handling primary repo prepending, duplicate-primary skipping, empty entry filtering, whitespace trimming, and default test command fallback. Despite this complexity, it has no test coverage. This task adds comprehensive unit tests covering all parsing branches and edge cases.

## Files to Modify

1. **`src/config.zig`** — Add test blocks after the existing `findEnvValue` tests (after line 270). The tests call the file-private `parseWatchedRepos` function directly, following the same pattern as the existing `findEnvValue` tests.

## Files to Create

None.

## Function/Type Signatures

No new functions or types are created. The tests call existing private functions:

```zig
fn parseWatchedRepos(
    allocator: std.mem.Allocator,
    env_content: []const u8,
    primary_repo: []const u8,
    primary_test_cmd: []const u8,
) ![]RepoConfig
```

Each test block follows this pattern:

```zig
test "parseWatchedRepos: <description>" {
    const alloc = std.testing.allocator;
    const env = \\WATCHED_REPOS=<value>
    ;
    const repos = try parseWatchedRepos(alloc, env, "<primary>", "<cmd>");
    defer alloc.free(repos);
    // For entries whose path/test_cmd were duped by the function:
    defer for (repos) |r| {
        if (!r.is_self) {
            alloc.free(r.path);
            if (!std.mem.eql(u8, r.test_cmd, "make test")) alloc.free(r.test_cmd);
        }
    };
    // assertions...
}
```

Note: Memory management in tests must account for the fact that `parseWatchedRepos` uses `allocator.dupe` for non-primary repo paths and non-default test commands, and returns an owned slice from `toOwnedSlice()`. The primary repo's `.path` and `.test_cmd` are NOT duped (they reference the passed-in slices directly). The default test command `"make test"` is a string literal, not heap-allocated.

## Acceptance Criteria

1. **Primary repo first**: When `primary_repo` is non-empty, `repos[0]` has `.path == primary_repo`, `.test_cmd == primary_test_cmd`, and `.is_self == true`.

2. **Empty primary repo**: When `primary_repo` is `""`, the primary repo is not added and the result contains only watched repos.

3. **Multiple pipe-delimited repos**: Input `"/a:cmd_a|/b:cmd_b"` produces two watched entries (plus primary if set) with correct paths and commands.

4. **Entry without colon uses default cmd**: Input `"/repo/path"` (no colon) produces an entry with `.test_cmd == "make test"`.

5. **Entry with colon but empty cmd uses default**: Input `"/repo/path:"` produces an entry with `.test_cmd == "make test"`.

6. **Duplicate primary is skipped**: If a watched entry's path matches `primary_repo`, it is not added a second time.

7. **Empty entries are skipped**: Input `"||"` or `"|/a:cmd|"` does not produce entries for the empty segments.

8. **Whitespace-only entries are skipped**: Input `"  | \t |/a:cmd"` skips the whitespace-only segments.

9. **Leading/trailing whitespace on paths and commands is trimmed**: Input `"  /path : cmd  "` produces `.path == "/path"` and `.test_cmd == "cmd"`.

10. **Entry with empty path after colon is skipped**: Input `":cmd"` (empty path) produces no entry.

11. **No WATCHED_REPOS in env**: When env_content has no `WATCHED_REPOS` line, only the primary repo (if set) is returned.

12. **All watched entries have `.is_self == false`**: Non-primary entries always have `is_self` set to `false`.

13. **Build and tests pass**: `zig build test` passes with all new and existing tests.

## Edge Cases

1. **Empty env_content and empty primary_repo**: Both are empty strings — result should be an empty slice (`repos.len == 0`).

2. **WATCHED_REPOS is empty string**: `WATCHED_REPOS=` — no watched repos parsed, only primary (if set).

3. **Single entry without delimiter**: `WATCHED_REPOS=/single:test` — one watched repo, no pipe splitting needed.

4. **Duplicate primary without colon**: `WATCHED_REPOS=/primary` where primary_repo is `/primary` — the entry matches and is skipped.

5. **Duplicate primary with colon**: `WATCHED_REPOS=/primary:other_cmd` where primary_repo is `/primary` — the entry matches by path and is skipped (the alternate command is ignored).

6. **Path with colon in command portion**: `WATCHED_REPOS=/repo:make -C /path test` — the split is on the first colon only (`std.mem.indexOf` returns the first match), so path is `/repo` and cmd is `make -C /path test`.

7. **Multiple consecutive pipes**: `WATCHED_REPOS=/a:x|||/b:y` — the empty segments between pipes are skipped.

8. **Whitespace around path with no colon and matching primary**: `WATCHED_REPOS=  /primary  ` where primary_repo is `/primary` — after trimming, matches primary and is skipped.

9. **Entry that is only a colon**: `WATCHED_REPOS=:` — path is empty after split, entry is skipped.

10. **Memory correctness**: Tests use `std.testing.allocator` (which detects leaks) and properly free all heap-allocated strings returned by `parseWatchedRepos` — the owned slice from `toOwnedSlice()`, duped paths, and duped non-default test commands.
