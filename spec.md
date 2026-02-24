# Spec: Extract duplicated SQLite parameter binding logic into shared helper

## Task Summary

The `query()` function (lines 106-128) and `execute()` function (lines 165-187) in `src/sqlite.zig` contain identical 22-line `inline for` blocks that bind tuple parameters to SQLite prepared statements. This duplication means any future type-support change (e.g. adding float or blob binding) must be made in two places. Extract the shared logic into a single `inline fn bindParams` that both functions call.

## Files to Modify

1. **`src/sqlite.zig`** — Add `bindParams` helper, replace duplicated inline-for blocks in `query()` and `execute()` with calls to it.

## Files to Create

None.

## Function/Type Signatures

### `src/sqlite.zig`

#### `bindParams` — new private inline function

```zig
inline fn bindParams(stmt: *c.sqlite3_stmt, params: anytype) SqliteError!void
```

- Accepts a non-null `*c.sqlite3_stmt` (caller must unwrap the optional before calling) and a tuple of parameters.
- Iterates over `params` with `inline for` and binds each element using the same logic currently duplicated in `query()` and `execute()`:
  - `isStringType` types → `sqlite3_bind_text` with `SQLITE_TRANSIENT`
  - `.int` / `.comptime_int` types → `sqlite3_bind_int64`
  - `.optional` types → unwrap: bind inner value (text or int64), or `sqlite3_bind_null` if `null`
  - Returns `SqliteError.BindFailed` if any bind call returns non-`SQLITE_OK`.
- This is a standalone `inline fn` at module scope (not a method on `Database`), since it only needs the statement handle and params, not `self`.

#### `Database.query` — modify (lines 96-154)

Replace lines 105-128 (the `// Bind parameters` comment and the `inline for` block) with:

```zig
try bindParams(stmt.?, params);
```

No other changes to `query()`.

#### `Database.execute` — modify (lines 156-194)

Replace lines 165-187 (the `inline for` block) with:

```zig
try bindParams(stmt.?, params);
```

No other changes to `execute()`.

## Acceptance Criteria

1. **Single definition**: The `inline for` parameter-binding logic exists exactly once in `src/sqlite.zig`, inside the `bindParams` function. Neither `query()` nor `execute()` contain an `inline for` over `params`.
2. **Behavioral equivalence**: `bindParams` handles the same type cases as the original code — string types (`isStringType`), integer types (`.int`, `.comptime_int`), and optional types (unwrapping to text/int64 or binding null). The binding index calculation (`i + 1`) is preserved.
3. **Error propagation**: `bindParams` returns `SqliteError.BindFailed` on any failed bind, and both `query()` and `execute()` propagate this error via `try`.
4. **Build succeeds**: `zig build` compiles without errors or warnings.
5. **Tests pass**: `zig build test` passes. All existing callers of `query()` and `execute()` (in `src/db.zig` and elsewhere) continue to work without modification.
6. **No public API change**: `Database.query` and `Database.execute` retain their existing public signatures. `bindParams` is a private module-level function (not `pub`).

## Edge Cases

1. **Empty params tuple**: Calling `query(alloc, sql, .{})` or `execute(sql, .{})` with an empty tuple must still work — `bindParams` with an empty tuple is a no-op (the `inline for` iterates zero times).
2. **Optional null values**: `bindParams` must correctly call `sqlite3_bind_null` when an optional parameter is `null`, same as the current code.
3. **Mixed parameter types**: A call like `execute(sql, .{ "text", 42, @as(?[]const u8, null) })` with mixed string, integer, and null-optional params must bind all three correctly in order.
4. **Comptime int literals**: Parameters like `.{ 1, 2 }` (comptime_int) must continue to work — the `.comptime_int` check in `@typeInfo(T)` must be preserved.
5. **String-coercible types**: Pointer-to-array types (e.g. `*const [5]u8` from string literals) must still be handled via `isStringType` and coerced to `[]const u8`.
6. **`rc` variable scoping**: The current code reuses the outer `var rc` from `prepare_v2` for binding results. The new `bindParams` function must use its own local `rc` variable (or check return codes inline), since it won't have access to the caller's `rc`. The caller's `rc` variable remains available for post-bind use (e.g. `sqlite3_step`).
