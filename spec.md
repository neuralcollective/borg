# Spec: Add tests for `getEnv` parsing of .env file content

## Task Summary

The `getEnv` function in `src/config.zig` (line 161) parses `.env` file content into key-value pairs and falls back to process environment variables, but has no direct tests. Existing tests only cover the internal `findEnvValue` helper. Tests must be added for `getEnv` itself to verify basic `KEY=VALUE` parsing, comment and blank line skipping, values containing `=` characters, and the process environment fallback path.

## Files to Modify

- `src/config.zig` — Append new `test` blocks after the existing tests (after line 267). Tests must live in this file because `getEnv` is a private (non-`pub`) function and is only accessible from within the same file.

## Files to Create

None.

## Function/Type Signatures

No new functions or types are created. The following existing private function is the test target:

```zig
/// Defined at src/config.zig:161
fn getEnv(allocator: std.mem.Allocator, env_content: []const u8, key: []const u8) ?[]const u8
```

Each new test block is a standard Zig `test "..." { ... }` block calling `getEnv` directly with `std.testing.allocator` and inline `.env`-formatted string content.

## Acceptance Criteria

### AC1: Basic `KEY=VALUE` parsing via `getEnv`
- Calling `getEnv(alloc, "MY_KEY=my_value", "MY_KEY")` returns `"my_value"`.
- The returned slice is an allocator-owned copy (must be freeable via `alloc.free`).
- Calling `getEnv(alloc, "MY_KEY=my_value", "NONEXISTENT")` returns `null` (assuming the key is not set in the process environment either; use an intentionally obscure key name like `"_BORG_TEST_MISSING_KEY_42"` to avoid collisions).

### AC2: Skipping `#` comment lines
- Given env content with comment lines (`# this is a comment`) interspersed with valid entries, `getEnv` returns the correct value for valid keys and `null` for keys that only appear in comments.
- A line like `# SECRET=hidden` must not be parseable as key `SECRET`.

### AC3: Skipping blank lines
- Given env content with empty lines and whitespace-only lines between valid entries, `getEnv` correctly parses and returns values for the valid keys.

### AC4: Values containing `=` characters
- Calling `getEnv(alloc, "TOKEN=abc=def=ghi", "TOKEN")` returns `"abc=def=ghi"`.
- Only the first `=` on a line is treated as the key-value separator; all subsequent `=` characters are part of the value.

### AC5: Process environment fallback
- When the key is absent from `env_content` but present in the process environment (via `std.posix.getenv`), `getEnv` returns the process environment value.
- When the key is present in both `env_content` and the process environment, the `.env` file value takes precedence.

### AC6: All tests pass
- Running `zig build test` completes with zero failures, including all existing tests and all new tests.

## Edge Cases to Handle

1. **Value with leading/trailing `=`**: e.g., `KEY==value=` should return `=value=` (everything after the first `=`).
2. **Key present in env content with empty value**: e.g., `EMPTY_VAL=` — `getEnv` should return `""` (empty string), not `null`.
3. **Whitespace around key and `=`**: e.g., `  MY_KEY  =  my_value  ` — `getEnv` should return `"my_value"` (whitespace trimmed from both key and value) per the existing `findEnvValue` trimming logic.
4. **Quoted values containing `=`**: e.g., `QUOTED="abc=def"` — `getEnv` should return `abc=def` (quotes stripped, `=` preserved in value).
5. **Process env fallback returns allocator-owned memory**: The returned slice from the process env path must be freeable via `alloc.free` without double-free or use-after-free (verified by using `std.testing.allocator` which detects leaks).
6. **Multiple entries with same structure**: env content with multiple valid lines should allow independent lookup of each key via separate `getEnv` calls.
