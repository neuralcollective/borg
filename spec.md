# Spec: Add tests for json.zig escapeString with special and control characters

## Task Summary

The `escapeString` function in `src/json.zig` (lines 61-80) has minimal test coverage: two existing tests cover a combined special-character string and two control characters (0x01, 0x1f), but miss `\r`, empty string, standalone character cases, multi-byte UTF-8, and the full range of control characters. Add targeted tests that verify each escape path individually, test multi-byte UTF-8 passthrough, and cover the empty-string edge case.

## Files to Modify

1. **`src/json.zig`** — Add new `test` blocks in the existing test section (after line 123) for `escapeString`.

## Files to Create

None.

## Function/Type Signatures

No new functions or types. Only new `test` blocks are added. Each test follows the existing pattern:

```zig
test "escapeString <description>" {
    const alloc = std.testing.allocator;
    const result = try escapeString(alloc, <input>);
    defer alloc.free(result);
    try std.testing.expectEqualStrings(<expected>, result);
}
```

## Acceptance Criteria

1. **Empty string**: `escapeString(alloc, "")` returns `""` (zero-length slice).
2. **Double quote**: `escapeString(alloc, "\"")` returns `"\\\""` (the two-byte sequence `\"`).
3. **Backslash**: `escapeString(alloc, "\\")` returns `"\\\\"` (the two-byte sequence `\\`).
4. **Newline**: `escapeString(alloc, "\n")` returns `"\\n"`.
5. **Carriage return**: `escapeString(alloc, "\r")` returns `"\\r"`.
6. **Tab**: `escapeString(alloc, "\t")` returns `"\\t"`.
7. **Control characters below 0x20**: Each control character not handled by a dedicated switch arm (e.g., 0x00 null, 0x07 bell, 0x0C form feed) produces `\u` followed by a 4-digit zero-padded hex code. Specifically test:
   - `0x00` → `\u0000`
   - `0x07` → `\u0007`
   - `0x0C` → `\u000c`
   - `0x1F` → `\u001f`
8. **Normal ASCII passthrough**: Printable ASCII (e.g., `"hello world 123!@#"`) passes through unchanged — output equals input.
9. **Multi-byte UTF-8 passthrough**: A string containing multi-byte UTF-8 characters (e.g., `"héllo 世界"`) passes through byte-for-byte unchanged — no bytes are escaped since all bytes ≥ 0x20.
10. **Mixed content**: A string combining normal text, special characters, and control characters in one input produces the correct combined escaped output.
11. **All tests pass**: `zig build test` succeeds with all new and existing tests passing.
12. **No memory leaks**: All tests use `std.testing.allocator` (which detects leaks) and `defer alloc.free(result)`.

## Edge Cases

1. **Empty string** — `escapeString` must not crash or allocate garbage; it must return an empty slice.
2. **Null byte (0x00)** — This is a valid control character that must be escaped as `\u0000`, not cause string truncation.
3. **Boundary control characters** — `0x1F` is the highest control character (should be escaped); `0x20` (space) is the first non-control character (should pass through unchanged). Test both to verify the `ch < 0x20` boundary.
4. **Multi-byte UTF-8 bytes** — Individual bytes of multi-byte UTF-8 sequences are all ≥ 0x80, so they pass the `ch < 0x20` check and must be appended verbatim. Verify the output length matches the input length for a UTF-8 string.
5. **All five switch arms** — Ensure each of the five explicit switch cases (`"`, `\`, `\n`, `\r`, `\t`) is tested individually, not only in combination, to avoid masking bugs where one arm's output accidentally satisfies an assertion meant for another.
6. **Carriage return specifically** — The existing tests cover `\n` and `\t` but not `\r`; this is the primary gap in the current coverage.
