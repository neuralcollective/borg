# Task #11 — Add tests for `getEnv` / `findEnvValue` `.env` parsing in config.zig

## 1. Task Summary

`getEnv` in `src/config.zig` drives all application configuration by reading key-value
pairs from `.env` file content via the private helper `findEnvValue`.  Existing tests
cover `findEnvValue` for basic `KEY=VALUE`, whitespace-trimmed values, quote-stripping,
and comment/blank-line skipping, but the case of a **value that itself contains `=`**
(e.g. `TOKEN=abc=def`) is not covered and the public entry-point `getEnv` has no direct
tests at all.  Silent mis-parsing of such values would misconfigure the entire
application with no error.

## 2. Files to Modify

| File | Reason |
|------|--------|
| `src/config.zig` | Add new `test` blocks inside the existing `// ── Tests ──` section at the bottom of the file. Both `getEnv` and `findEnvValue` are `fn`-private; tests must live in the same file to access them. |

## 3. Files to Create

None.  All new tests belong in the existing test section of `src/config.zig`.

## 4. Function / Type Signatures for New or Changed Code

No new functions or types are introduced.  The additions are pure test blocks that call
the existing private functions:

```zig
// Existing private targets under test (no signature change):
fn findEnvValue(allocator: std.mem.Allocator, content: []const u8, key: []const u8) ?[]const u8
fn getEnv(allocator: std.mem.Allocator, env_content: []const u8, key: []const u8) ?[]const u8

// New test blocks to add (names are requirements, not suggestions):
test "findEnvValue value containing equals sign returns full remainder"
test "getEnv basic KEY=VALUE from env content"
test "getEnv skips hash comment lines"
test "getEnv skips blank lines"
test "getEnv value with embedded equals sign"
test "getEnv returns null when key absent from content and process env"
test "getEnv env file value takes precedence when key exists in content"
```

Each test block must follow the project's existing style:
- allocator: `std.testing.allocator`
- `defer` every allocation returned by the function under test
- assertions via `std.testing.expectEqualStrings` and `std.testing.expect`

## 5. Acceptance Criteria

All criteria must pass under `zig build test` (`just t`) with zero regressions.

**AC-1 — `findEnvValue`: value containing `=`**
```
env_content = "TOKEN=abc=def"
findEnvValue(alloc, env_content, "TOKEN") == "abc=def"
```
Parsing must split on the *first* `=` only; everything after it is the value.

**AC-2 — `getEnv`: basic `KEY=VALUE`**
```
env_content = "KEY=value"
getEnv(alloc, env_content, "KEY") == "value"
```

**AC-3 — `getEnv`: comment lines are skipped**
```
env_content = "# this is a comment\nREAL=found"
getEnv(alloc, env_content, "REAL") == "found"
getEnv(alloc, env_content, "#")    == null   // comment marker is not a key
```

**AC-4 — `getEnv`: blank lines are skipped**
```
env_content = "\n\nKEY=value"
getEnv(alloc, env_content, "KEY") == "value"
```

**AC-5 — `getEnv`: value with embedded `=`**
```
env_content = "TOKEN=abc=def"
getEnv(alloc, env_content, "TOKEN") == "abc=def"
```

**AC-6 — `getEnv`: returns null when key is absent**
```
env_content = "OTHER=x"
// Key must also be absent from the real process environment.
// Use a key guaranteed not to be set, e.g. "BORG_TEST_MISSING_KEY_XYZ_11".
getEnv(alloc, env_content, "BORG_TEST_MISSING_KEY_XYZ_11") == null
```

**AC-7 — `getEnv`: env file value is returned when key is present in content**
```
env_content = "BORG_TEST_KEY_11=from_file"
getEnv(alloc, env_content, "BORG_TEST_KEY_11") == "from_file"
```
(This key is not expected to be set in the process environment; the test verifies the
file-read path returns the correct value without requiring environment mutation.)

**AC-8 — all new tests are leak-free**
Every allocation returned by `getEnv` / `findEnvValue` is freed with `defer`.
Running under `std.testing.allocator` reports zero leaked bytes for each test.

## 6. Edge Cases to Handle

| Edge case | Expected behaviour |
|-----------|-------------------|
| Value is the empty string: `KEY=` | `getEnv` returns `""` (empty slice, not null) |
| Value contains multiple `=`: `A=x=y=z` | Returns `"x=y=z"` — split on first `=` only |
| Indented comment `  # comment` | Still treated as a comment; the line is skipped |
| Windows-style line ending `\r\n` | `\r` is stripped by the existing `trim` call; returned value must not include `\r` |
| Value in double quotes containing `=`: `KEY="a=b"` | Returns `"a=b"` — quotes are stripped and embedded `=` is preserved |
| Key with surrounding whitespace: `KEY =value` | Key trimmed to `"KEY"`; matches and returns `"value"` |
| Key appears more than once in content | Returns the value from the **first** matching line (existing behaviour — test documents it, does not change it) |
| Line consisting only of `=` | Empty key `""` never matches any real key lookup; safely skipped |
