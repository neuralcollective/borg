# Task #6: Fix OAuth Token Memory Leak in `refreshOAuthToken`

## 1. Task Summary

`Config.refreshOAuthToken` (src/config.zig:114-118) allocates a new heap string via
`readOAuthToken` and overwrites `self.oauth_token` without freeing the previous value.
Because this function is called on every main-loop iteration (~500 ms, src/main.zig:715)
and also before each pipeline agent spawn (src/pipeline.zig:327, 1303, 1321), the
process leaks memory continuously at runtime. The fix introduces an `oauth_token_owned`
boolean flag on `Config` to track whether the current `oauth_token` is heap-allocated,
and frees the old token before replacing it only when the flag is set.

---

## 2. Files to Modify

| File | Change |
|---|---|
| `src/config.zig` | Add `oauth_token_owned: bool` field to `Config`; update `Config.load` to set it correctly; update `refreshOAuthToken` to free old token when owned and set flag to `true` after successful refresh; add `test { _ = @import("config_test.zig"); }` to wire tests into the build |
| `src/config_test.zig` | Add missing `.max_pipeline_agents = 0` field to `testConfig` helper (compile error without it, since `Config` has this field) |

---

## 3. Files to Create

None. `src/config_test.zig` already exists and contains all required tests.

---

## 4. Function / Type Signatures for New or Changed Code

### 4.1 `Config` struct — new field (src/config.zig)

```zig
pub const Config = struct {
    telegram_token: []const u8,
    oauth_token: []const u8,
    oauth_token_owned: bool,   // true iff oauth_token was heap-allocated by this Config
    // ... all remaining existing fields unchanged ...
};
```

`oauth_token_owned` must be placed immediately after `oauth_token` (or anywhere before
`allocator`) so that every struct literal in tests and in `Config.load` can initialize it.

### 4.2 `Config.load` — initialize `oauth_token_owned` (src/config.zig)

The `oauth` variable is non-empty if and only if it was heap-allocated by `readOAuthToken`
or `getEnv`. Set the flag accordingly:

```zig
var config = Config{
    // ...
    .oauth_token = oauth,
    .oauth_token_owned = (oauth.len > 0),
    // ...
};
```

### 4.3 `Config.refreshOAuthToken` — free old token, set ownership (src/config.zig)

```zig
/// Re-read OAuth token from credentials file (handles token rotation).
pub fn refreshOAuthToken(self: *Config) void {
    if (readOAuthToken(self.allocator, self.credentials_path)) |new_token| {
        if (self.oauth_token_owned) {
            self.allocator.free(self.oauth_token);
        }
        self.oauth_token = new_token;
        self.oauth_token_owned = true;
    }
}
```

Signature is unchanged: `pub fn refreshOAuthToken(self: *Config) void`.

### 4.4 `testConfig` helper — add missing field (src/config_test.zig)

Add `.max_pipeline_agents = 0,` after `.rate_limit_per_minute = 0,` in the `testConfig`
function's struct literal so it matches the current `Config` definition.

### 4.5 Wire `config_test.zig` into the test build (src/config.zig)

Append after the last existing `test` block in `src/config.zig`:

```zig
test {
    _ = @import("config_test.zig");
}
```

---

## 5. Acceptance Criteria

All criteria are directly verified by tests in `src/config_test.zig`:

| ID | Assertion |
|---|---|
| AC1 | After `refreshOAuthToken` when `readOAuthToken` succeeds, the previous heap-allocated `oauth_token` is freed. `std.testing.allocator` reports no leak. |
| AC2 | When `oauth_token_owned = false` (initial token is the `""` literal), `refreshOAuthToken` does **not** call `allocator.free` on the old value — no crash or double-free. |
| AC3 | After a successful refresh, `cfg.oauth_token` equals the new token string (no use-after-free corruption). |
| AC4 | When `readOAuthToken` returns `null` (file missing or unreadable), `refreshOAuthToken` is a no-op: `oauth_token` and `oauth_token_owned` are unchanged. |
| AC5 | Multiple consecutive refreshes produce no memory leaks detected by `std.testing.allocator`. |
| Structural | `Config` compiles with the new `oauth_token_owned: bool` field and `testConfig` in `config_test.zig` compiles without error. |
| Build | `zig build test` exits 0 with all tests passing (no regressions). |

---

## 6. Edge Cases to Handle

1. **Initial token is the empty string literal `""`** — `oauth_token_owned` must be
   `false` in `Config.load` (set via `oauth.len > 0`). The first `refreshOAuthToken`
   call must not attempt `allocator.free("")`.

2. **Initial token is heap-allocated (non-empty)** — When `readOAuthToken` or `getEnv`
   produces a non-empty slice, `oauth_token_owned = true` ensures the first refresh
   frees it correctly.

3. **Credentials file disappears mid-run** — `readOAuthToken` returns `null`;
   `refreshOAuthToken` must be a strict no-op, leaving both `oauth_token` and
   `oauth_token_owned` unchanged.

4. **Credentials file contains invalid JSON or is missing the `accessToken` field** —
   `readOAuthToken` returns `null`; same no-op behaviour as edge case 3.

5. **Token value is identical across refreshes** — Even if the content is byte-for-byte
   identical, `readOAuthToken` allocates a new slice each call. The old allocation must
   still be freed to avoid a leak.

6. **`oauth_token_owned` transitions `false → true` on first successful refresh** — After
   that point all subsequent refreshes must free the old value before assigning the new
   one. The test `"oauth_token_owned transitions from false to true on first successful
   refresh"` in `config_test.zig` covers the full transition sequence.

7. **`testConfig` compile fix** — `src/config_test.zig:testConfig` does not include
   `.max_pipeline_agents`, which is a required field of `Config`. This must be added to
   resolve the compile error that would otherwise block all test runs.
