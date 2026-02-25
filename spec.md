# Task #25: Add tests for json accessor functions with non-object input

## 1. Task Summary

The five JSON accessor functions (`getString`, `getInt`, `getBool`, `getObject`, `getArray`) in `src/json.zig` all guard against non-object input with `if (obj != .object) return null`, but no test passes a non-object `Value` as the top-level `obj` argument to any of these functions. This task adds tests that verify each accessor returns `null` when `obj` is not an object, and covers the float-to-int coercion path in `getInt` (line 28).

## 2. Files to Modify

- `src/json.zig` — append new test blocks after the existing tests (after line 134)

## 3. Files to Create

None.

## 4. Function/Type Signatures for New or Changed Code

No new exported functions or types. Two new test blocks are to be added at the bottom of `src/json.zig`:

```zig
test "accessor functions return null for non-object input" { ... }
test "getInt handles float-to-int coercion" { ... }
```

## 5. Acceptance Criteria

1. **`getString` with non-object `obj`**: `getString(Value{ .string = "hello" }, "key")` returns `null`.
2. **`getInt` with non-object `obj`**: `getInt(Value{ .string = "hello" }, "key")` returns `null`.
3. **`getBool` with non-object `obj`**: `getBool(Value{ .string = "hello" }, "key")` returns `null`.
4. **`getObject` with non-object `obj`**: `getObject(Value{ .string = "hello" }, "key")` returns `null`.
5. **`getArray` with non-object `obj`**: `getArray(Value{ .string = "hello" }, "key")` returns `null`.
6. **Float-to-int coercion in `getInt`**: Parsing `{"x":3.0}` and calling `getInt(parsed.value, "x")` returns a non-null result equal to `@as(i64, 3)`.
7. **`zig build test` passes** with all existing and new tests green.

## 6. Edge Cases to Handle

- **Choice of non-object Value**: Use `Value{ .string = "s" }` as the non-object input; it can be constructed without an allocator or `defer`, keeping the test simple. An array `Value` may also be used to confirm the guard is not tag-specific, but is not required.
- **Float coercion requires parsing**: To produce a `Value` with a `.float` variant, use `parse(alloc, "{\"x\":3.0}")` with `defer parsed.deinit()`. Direct construction of `Value{ .float = 3.0 }` wrapped in an object is also acceptable.
- **Exact integer float only**: The float test value must be an exact integer representable as a float (e.g. `3.0`), so the coerced result is unambiguous. Do not test truncation of fractional values (e.g. `3.9`) — that would be a separate concern outside the scope of this task.
- **No allocator needed for non-object tests**: `Value{ .string = "..." }` can be stack-constructed, so no allocator or `defer` is required in the non-object test block.
