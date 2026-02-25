# Spec: Add tests for json.stringify in json.zig

## 1. Task Summary

`json.stringify` (src/json.zig:83) converts a `std.json.Value` back to a JSON string and is used for serialization throughout the codebase, but currently has zero test coverage. This task adds four focused test blocks directly in `src/json.zig` covering a simple string value, null, an object with mixed types, and nested objects/arrays with round-trip verification via `parse`.

## 2. Files to Modify

- `src/json.zig` — append four new `test` blocks after the existing tests (after line 134)

## 3. Files to Create

None.

## 4. Function/Type Signatures for New or Changed Code

No new public functions or types are introduced. The additions are four private `test` blocks within `src/json.zig`:

```zig
test "stringify simple string value" { ... }

test "stringify null" { ... }

test "stringify object with mixed types round-trips with parse" { ... }

test "stringify nested objects and arrays round-trips with parse" { ... }
```

Each test follows this pattern:
```zig
const alloc = std.testing.allocator;
// ... build or parse a Value ...
const result = try stringify(alloc, value);
defer alloc.free(result);
// ... assertions ...
```

For tests that use `parse`, the parsed value must be released:
```zig
var parsed = try parse(alloc, json_input);
defer parsed.deinit();
```

## 5. Acceptance Criteria

1. **Simple string value**: Calling `stringify(alloc, Value{ .string = "hello" })` returns the slice `"\"hello\""` (i.e., the JSON encoding of the string `hello`).

2. **Null value**: Calling `stringify(alloc, .null)` (or `Value{ .null = {} }`) returns the slice `"null"`.

3. **Object with mixed types — round-trip**: A `std.json.Value` of type `.object` is constructed in-memory with at least the following fields:
   - `"name"`: string `"borg"`
   - `"count"`: integer `1`
   - `"active"`: bool `true`

   After calling `stringify`, the returned JSON string is fed back into `parse`. The re-parsed value must satisfy:
   - `getString(reparsed.value, "name").?` equals `"borg"`
   - `getInt(reparsed.value, "count").?` equals `1`
   - `getBool(reparsed.value, "active").?` equals `true`

4. **Nested objects and arrays — round-trip**: The JSON string `"{\"outer\":{\"inner\":\"val\"},\"list\":[1,2,3]}"` is parsed with `parse`, passed to `stringify`, then re-parsed with `parse`. The final parsed value must satisfy:
   - `getObject(reparsed.value, "outer")` is non-null
   - `getString(getObject(reparsed.value, "outer").?, "inner").?` equals `"val"`
   - `getArray(reparsed.value, "list").?` has length `3`

5. **Memory safety**: All four tests pass under `zig build test` with no memory leaks detected by `std.testing.allocator`.

6. **No regressions**: All pre-existing tests in `src/json.zig` (`parse and access typed fields`, `escapeString handles special characters`, `escapeString handles control characters`, `getString returns null for missing key and wrong type`) continue to pass.

## 6. Edge Cases to Handle

- **Memory ownership of `stringify` result**: The returned slice is caller-owned. Every test must `defer alloc.free(result)` immediately after the `stringify` call.
- **Memory ownership of `parse` result**: Every `parse` call in tests must have a matching `defer parsed.deinit()` to free the arena allocator backing the parsed tree.
- **Object key ordering is not guaranteed**: The object-with-mixed-types test must not assert the exact byte content of the stringified object. Instead, it must re-parse the result and check field values individually, because `std.json.ObjectMap` (backed by `std.StringArrayHashMap`) may serialize keys in insertion order, which can vary across Zig versions.
- **In-memory `Value` construction for the object test**: Building a `Value{ .object = ... }` requires initializing a `std.json.ObjectMap` with the test allocator, inserting fields, and ensuring the map is freed after the test. Use `defer obj.deinit()` where `obj` is the `ObjectMap`. Alternatively, parse a JSON literal string to obtain the `Value`, which handles memory automatically via the `Parsed` arena.
- **Float fields**: If a float field is added to future tests, use approximate comparison (`expectApproxEqAbs`) rather than exact equality, since `std.json.stringify` renders floats with finite precision and a subsequent parse may yield a slightly different `f64`.
- **Integer boundary**: Any integer values used in tests must be representable as `i64` without overflow (the type used by `std.json.Value.integer`).
- **Non-empty string**: The simple-string test must use a non-empty string to distinguish correct output from an accidental empty or null result.
