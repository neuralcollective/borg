# Spec: Add tests for json.zig safe getter functions

## Task Summary

The `getString`, `getInt`, and `getBool` functions in `src/json.zig` lack dedicated tests for missing-key and wrong-type scenarios. The existing test ("parse and access typed fields") only covers the happy path where keys are present with the correct type. Add focused tests that verify each getter returns `null` when the key is absent, when the value is a mismatched type, and when called on a non-object value, and returns the correct value when the key is present.

## Files to Modify

1. **`src/json.zig`** — Add new `test` blocks after the existing tests (after line 134).

## Files to Create

None.

## Function/Type Signatures

No new functions or types. Only new `test` blocks are added within `src/json.zig`.

### New test blocks

```zig
test "getString returns correct value for present key" { ... }
test "getString returns null for missing key" { ... }
test "getString returns null for wrong value type" { ... }
test "getString returns null for non-object value" { ... }

test "getInt returns correct value for present key" { ... }
test "getInt returns null for missing key" { ... }
test "getInt returns null for wrong value type" { ... }
test "getInt coerces float to int" { ... }
test "getInt returns null for non-object value" { ... }

test "getBool returns correct value for present key" { ... }
test "getBool returns null for missing key" { ... }
test "getBool returns null for wrong value type" { ... }
test "getBool returns null for non-object value" { ... }
```

Each test block uses `std.testing.allocator`, calls `json.parse` to create a `Parsed(Value)`, defers `.deinit()`, and asserts results with `std.testing.expectEqualStrings`, `std.testing.expectEqual`, or `std.testing.expect`.

Note: Some of these cases are partially covered by the existing `"getString returns null for missing key and wrong type"` test (line 125). The new tests should be more comprehensive and cover `getInt` and `getBool` symmetrically, but the existing test can be left as-is or consolidated at the implementer's discretion.

## Acceptance Criteria

1. **Missing key returns null**: For each of `getString`, `getInt`, `getBool`, calling the function with a key not present in the parsed JSON object returns `null`.
2. **Present key returns correct value**: `getString` returns the string value, `getInt` returns the `i64` value, `getBool` returns the `bool` value when the key exists and has the matching type.
3. **Wrong type returns null**: `getString` returns `null` when the key maps to an integer or bool. `getInt` returns `null` when the key maps to a string or bool. `getBool` returns `null` when the key maps to a string or integer.
4. **Non-object input returns null**: Calling each getter on a non-object `Value` (e.g., a `Value` that is `.string` or `.integer`) returns `null`.
5. **Float-to-int coercion**: `getInt` returns the truncated `i64` when the value is a JSON float (e.g., `3.0` → `3`). This exercises the `.float => |f| @intFromFloat(f)` branch at line 28.
6. **`getBool` with both true and false**: Tests cover both `true` and `false` boolean values returning correctly.
7. **Build passes**: `zig build` succeeds with no errors.
8. **All tests pass**: `zig build test` passes, including both the new tests and all pre-existing tests.

## Edge Cases

1. **Non-object value as input**: All three getters have a guard `if (obj != .object) return null`. Test by passing a `Value` that is `.string`, `.integer`, or `.null` directly (not wrapped in an object).
2. **Float coercion in `getInt`**: The `getInt` function coerces `.float` values via `@intFromFloat`. Test with a value like `3.0` to confirm it returns `3`. This is a branch not covered by any existing test.
3. **Null JSON value for a present key**: A key that maps to JSON `null` (`.null` variant) should return `null` from all three getters since none match on `.null`.
4. **Empty object**: Parsing `{}` and calling any getter should return `null` for any key.
5. **Empty string value**: `getString` on a key whose value is `""` should return the empty string, not `null`.
6. **Negative and zero integers**: `getInt` should correctly return negative values and zero.
7. **`false` value for `getBool`**: Ensure `getBool` returns `false` (not `null`) for a key mapped to `false` — the optional return type means the test must distinguish `?bool` being `false` from being `null`.
