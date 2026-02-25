# Spec: Fix memory leak in Config.refreshOAuthToken

## Task Summary

`Config.refreshOAuthToken` (`src/config.zig:111`) replaces `self.oauth_token` with a newly heap-allocated string from `readOAuthToken` but never frees the previous value. Since this function is called every main-loop iteration (~500ms in `src/main.zig:709`), on every pipeline seed cycle (`src/pipeline.zig:208`), and on every agent spawn (`src/pipeline.zig:1085`), it causes steady memory growth proportional to uptime. The fix must free the old token before replacing it, while handling the case where the initial token was not heap-allocated (empty string literal `""`).

## Files to Modify

- `src/config.zig` — Fix `refreshOAuthToken` to free the old token; track whether `oauth_token` is heap-owned.

## Files to Create

None.

## Function/Type Signatures

### `Config` struct (`src/config.zig:9`)

Add a field to track whether the current `oauth_token` was heap-allocated and should be freed on replacement:

```zig
oauth_token_owned: bool,  // true when oauth_token was allocated by readOAuthToken and must be freed
```

### `Config.load` (`src/config.zig:46`)

Initialize `oauth_token_owned` based on whether `readOAuthToken` returned a value:

- If `readOAuthToken` returned non-null: set `oauth_token_owned = true`
- If it fell back to `getEnv`: set `oauth_token_owned = true` (getEnv also heap-allocates via `allocator.dupe`)
- If it fell through to the empty string literal `""`: set `oauth_token_owned = false`

### `Config.refreshOAuthToken` (`src/config.zig:111`)

Updated signature (unchanged, still `pub fn refreshOAuthToken(self: *Config) void`), but new body:

```
pub fn refreshOAuthToken(self: *Config) void {
    if (readOAuthToken(self.allocator, self.credentials_path)) |new_token| {
        if (self.oauth_token_owned) {
            self.allocator.free(@constCast(self.oauth_token));
        }
        self.oauth_token = new_token;
        self.oauth_token_owned = true;
    }
}
```

Key detail: `self.oauth_token` is typed `[]const u8` but the heap-allocated values came from `allocator.dupe(u8, ...)` which returns `[]u8`. Use `@constCast` to obtain the mutable slice needed by `allocator.free`. This is safe because we only free values we know were heap-allocated (guarded by `oauth_token_owned`).

## Acceptance Criteria

1. **Old token is freed**: After `refreshOAuthToken` is called with a new token available, the previous `oauth_token` memory is freed. Verifiable by running `zig build test` with `std.testing.allocator` (which detects leaks).

2. **No double-free on literal**: When `oauth_token` is initialized to `""` (a string literal, not heap-allocated) and `refreshOAuthToken` is called for the first time, no free is attempted on the literal. Verifiable by a test that creates a Config with `oauth_token_owned = false` and calls `refreshOAuthToken`.

3. **No use-after-free**: The old token pointer is not accessed after being freed. The assignment `self.oauth_token = new_token` happens after the free.

4. **No-op when token unchanged**: If `readOAuthToken` returns `null` (credentials file missing or unreadable), `oauth_token` is not modified and no free occurs. Existing behavior preserved.

5. **Unit test**: Add a test in `src/config.zig` that exercises `refreshOAuthToken` with `std.testing.allocator`:
   - Construct a `Config` with a heap-allocated `oauth_token` (`oauth_token_owned = true`).
   - Call `refreshOAuthToken` (will need a mock or a temp credentials file).
   - Verify no leak is reported by `std.testing.allocator`.
   - Alternatively, test the free logic directly: allocate a token, assign it, call refresh, confirm the allocator reports no leaks at scope exit.

6. **Existing tests pass**: `zig build test` passes with no regressions.

## Edge Cases

1. **Initial token is empty literal `""`**: The first call to `refreshOAuthToken` must not attempt to free the empty string literal. Handled by `oauth_token_owned = false` at init.

2. **Initial token from `getEnv` or `readOAuthToken`**: Both return heap-allocated memory. `oauth_token_owned` must be `true` so the first refresh frees it.

3. **Credentials file missing or invalid**: `readOAuthToken` returns `null`. `refreshOAuthToken` must be a no-op — no free, no reassignment.

4. **Credentials file returns same token value**: Even if the token content is identical, `readOAuthToken` allocates a new copy each call. The old copy must still be freed. (Optimization to skip replacement when content matches is out of scope but would be a valid follow-up.)

5. **Concurrent access**: `refreshOAuthToken` is called from the main loop and pipeline code. Per CLAUDE.md, SQLite uses WAL with a single connection, and the main loop is single-threaded. Confirm that `refreshOAuthToken` is only called from the main thread or that `Config` access is not shared across threads without synchronization. If it is thread-safe by design (single-threaded event loop), no mutex is needed.

6. **Allocator failure in `readOAuthToken`**: If `allocator.dupe` fails inside `readOAuthToken`, it returns `null`, and `refreshOAuthToken` is a no-op. The old token remains valid. No change needed.
