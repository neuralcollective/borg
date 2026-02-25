# Task #26: Add tests for decodeChunked edge cases in http.zig

## 1. Task Summary

`decodeChunked` (http.zig:155) currently has two happy-path tests covering normal multi-chunk and single-chunk inputs. This task adds five edge-case tests that exercise boundary and error conditions arising from real Docker API responses over Unix sockets, ensuring the function degrades gracefully without panicking or leaking memory.

## 2. Files to Modify

- `src/http.zig` — append five new `test` blocks after the existing tests at line 193

## 3. Files to Create

None. Because `decodeChunked` is a private (`fn`, not `pub fn`) function, tests must live in the same file and cannot be placed in a separate `*_test.zig` file without first exporting the function.

## 4. Function / Type Signatures for New or Changed Code

No signatures change. The five new test blocks call the existing private function:

```zig
fn decodeChunked(allocator: std.mem.Allocator, data: []const u8) ![]u8
```

Each test follows the pattern already established in the file:

```zig
test "decodeChunked <description>" {
    const alloc = std.testing.allocator;
    const result = try decodeChunked(alloc, <input>);
    defer alloc.free(result);
    try std.testing.expectEqualStrings(<expected>, result);
}
```

## 5. Acceptance Criteria

1. **Empty input** — `decodeChunked(alloc, "")` returns `""` without error.
   - `pos < data.len` is immediately false; the loop body never executes.
   - `result.toOwnedSlice()` returns a valid zero-length slice that can be freed without UB.

2. **Immediate zero-size chunk (empty body)** — `decodeChunked(alloc, "0\r\n\r\n")` returns `""` without error.
   - The function parses hex `"0"`, sees `chunk_size == 0`, and breaks before appending anything.

3. **Malformed hex chunk size** — `decodeChunked(alloc, "xyz\r\ndata\r\n0\r\n\r\n")` returns `""` without error.
   - `std.fmt.parseInt("xyz", 16)` returns an error; the `catch break` clause exits the loop cleanly.
   - The function must not propagate the parse error to the caller (i.e. `try decodeChunked(...)` must not fail).

4. **Truncated chunk data** — `decodeChunked(alloc, "a\r\nhello\r\n")` returns `""` without error.
   - Chunk size is `0xa = 10`; only 5 bytes of payload are present after the size line.
   - The `pos + chunk_size > data.len` guard fires, the loop breaks before appending, and an empty slice is returned.

5. **Missing `\r\n` between chunks** — `decodeChunked(alloc, "4\r\nWiki5\r\npedia\r\n0\r\n\r\n")` returns `"Wiki"` without error.
   - The first chunk ("Wiki", 4 bytes) is decoded correctly.
   - After consuming the first chunk, `pos` advances by `chunk_size + 2` assuming a `\r\n` separator that is absent, landing mid-stream.
   - The next size-line candidate is `"pedia"`, which is not valid hex, so `catch break` fires.
   - Result is the partial decode `"Wiki"`, demonstrating graceful truncation rather than a panic or memory fault.

All five tests must pass under `just t` (`zig build test`) with no memory leaks reported by `std.testing.allocator`.

## 6. Edge Cases to Handle

| Edge case | Expected behaviour |
|-----------|-------------------|
| Zero-length `toOwnedSlice` result | `ArrayList.toOwnedSlice()` on an empty list returns a valid (possibly zero-length) slice; `alloc.free(result)` must not fault. Tests 1, 2, 3, and 4 exercise this path. |
| `catch break` swallows parse error | `decodeChunked` signature is `![]u8` but malformed input must not cause an error return — only a graceful empty result. Tests 3 and 5 confirm this. |
| Partial decode on missing separator | When the inter-chunk `\r\n` is absent, data successfully decoded before the bad boundary is preserved in the return value (test 5). The function does not discard previously appended chunks. |
| Allocator leak detection | Every test must `defer alloc.free(result)` immediately after the `try decodeChunked(...)` call so that `std.testing.allocator` reports leaks as test failures. |
