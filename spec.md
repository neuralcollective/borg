# Spec: Add tests for `json.zig` `escapeString` with special and control characters

## Task Summary

Add comprehensive test coverage for the `escapeString` function in `src/json.zig`. Two tests already exist (lines 109-123) covering a combined special-character string (`"`, `\`, `\n`, `\t`) and two control characters (`0x01`, `0x1f`), but they miss `\r`, empty string input, normal ASCII passthrough, multi-byte UTF-8 passthrough, the null byte (`0x00`), and isolated per-character validation. New tests must fill these gaps so every code path in the function's `switch` statement and the `ch < 0x20` branch is individually verified.

## Files to Modify

- `src/json.zig` — Add new `test` blocks after the existing tests (after line 134).

## Files to Create

None.

## Function/Type Signatures for New or Changed Code

No new functions or types are introduced. All additions are Zig `test` blocks inside `src/json.zig` that call the existing public function:

```zig
pub fn escapeString(allocator: std.mem.Allocator, input: []const u8) ![]const u8
```

Each test block follows this pattern:

```zig
test "<descriptive name>" {
    const alloc = std.testing.allocator;
    const result = try escapeString(alloc, <input>);
    defer alloc.free(result);
    try std.testing.expectEqualStrings(<expected>, result);
}
```

### Test blocks to add

1. **`test "escapeString returns empty output for empty input"`**
   - Input: `""`
   - Expected output: `""`

2. **`test "escapeString escapes carriage return"`**
   - Input: `"\r"`
   - Expected output: `"\\r"`

3. **`test "escapeString passes through normal ASCII unchanged"`**
   - Input: `"Hello, world! 0123 ABC abc ~"` (printable ASCII including space, digits, letters, punctuation, tilde)
   - Expected output: identical to input

4. **`test "escapeString passes through multi-byte UTF-8 unchanged"`**
   - Input: `"café 日本語 🚀"` (2-byte, 3-byte, and 4-byte UTF-8 sequences)
   - Expected output: identical to input

5. **`test "escapeString escapes null byte"`**
   - Input: `&[_]u8{0x00}`
   - Expected output: `"\\u0000"`

6. **`test "escapeString escapes all control characters below 0x20"`**
   - Input: a byte array containing every value from `0x00` to `0x1f` (32 bytes)
   - Expected output: the 5 explicitly-handled characters produce their named escapes (`\t` for 0x09, `\n` for 0x0a, `\r` for 0x0d); the remaining 27 produce `\uXXXX` hex escapes
   - Verify total output length equals `5 * 2 + 27 * 6 = 172` characters (the 5 named escapes are 2 chars each like `\n`; the 27 hex escapes are 6 chars each like `\u0000`)
   - Note: `"` is 0x22 and `\` is 0x5c, both above 0x20, so they are NOT in this range

7. **`test "escapeString escapes each special character in isolation"`**
   - Sub-checks for `"`, `\`, `\n`, `\t`, `\r` each as a single-character input
   - Verifies each produces its two-character escape: `\"`, `\\`, `\n`, `\t`, `\r`

8. **`test "escapeString handles mixed content with all escape types"`**
   - Input: a string combining normal ASCII, a special character, a control character, and multi-byte UTF-8, e.g. `"hi\x01\t" ++ "é"`
   - Expected output: `"hi\\u0001\\té"` — verifying that escaping and passthrough work correctly when interleaved

## Acceptance Criteria

1. **AC1**: `escapeString(alloc, "")` returns a zero-length slice (empty string).
2. **AC2**: `escapeString(alloc, "\r")` returns `"\\r"` (two bytes: backslash, letter r).
3. **AC3**: `escapeString(alloc, <printable ASCII string>)` returns a slice byte-equal to the input.
4. **AC4**: `escapeString(alloc, <multi-byte UTF-8 string>)` returns a slice byte-equal to the input — no UTF-8 bytes are mangled or escaped.
5. **AC5**: `escapeString(alloc, &[_]u8{0x00})` returns `"\\u0000"` (6 bytes).
6. **AC6**: For every byte value `0x00..0x1f`, `escapeString` produces the correct escape: named escapes for `\t` (0x09), `\n` (0x0a), `\r` (0x0d), and `\uXXXX` hex escapes for all others. `"` (0x22) and `\` (0x5c) are not in this range.
7. **AC7**: Each of the five special characters (`"`, `\`, `\n`, `\t`, `\r`) in isolation produces exactly its two-character escape sequence.
8. **AC8**: A mixed input containing normal text, special characters, control characters, and UTF-8 produces the expected combined output with correct escaping and passthrough.
9. **AC9**: All new tests pass when run via `zig build test`.
10. **AC10**: All pre-existing tests in `src/json.zig` continue to pass (no regressions).

## Edge Cases to Handle

1. **Empty string**: Input `""` must return an allocated empty slice (length 0), not null or an error.
2. **Null byte (0x00)**: Must be escaped as `\u0000`, not silently dropped or treated as a string terminator (Zig slices are length-delimited, not null-terminated).
3. **Byte 0x1f (Unit Separator)**: The highest control character must still be escaped via the `ch < 0x20` path, confirming the boundary condition is `< 0x20` not `<= 0x1f` (equivalent, but worth verifying).
4. **Byte 0x20 (space)**: Must NOT be escaped — it is the first printable ASCII character and should pass through unchanged. This validates the boundary of the `ch < 0x20` check.
5. **Multi-byte UTF-8 bytes (0x80-0xFF)**: Individual bytes in multi-byte UTF-8 sequences are above 0x7F and must pass through the `else` branch unchanged. The function operates byte-by-byte, so it must not corrupt multi-byte sequences.
6. **Allocator failure**: The function signature returns `![]const u8` (error union). Tests use `std.testing.allocator` which detects leaks. Each test must `defer alloc.free(result)` to avoid leak detection failures.
7. **Adjacent special characters**: Input like `"\"\\"` (quote then backslash) must produce `"\\\"\\\\"` — verify that escape sequences from adjacent characters don't interfere with each other.
