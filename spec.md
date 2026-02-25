# Spec: Guard against negative values in Telegram mention offset/length casts

## Task Summary

In `src/telegram.zig` lines 108-109, `@intCast` converts `i64` values (from `json.getInt`) to `usize` without checking for negative numbers. A negative value from malformed Telegram entity data would cause a runtime panic at the `@intCast`. Add bounds checks before casting so that entities with negative offset or length values are safely skipped via `continue`.

## Files to Modify

1. **`src/telegram.zig`** — Add negative-value guards before the `@intCast` calls on lines 108-109.

## Files to Create

None.

## Function/Type Signatures

No new functions or types. The change is within the existing `for (entities) |entity|` loop body inside `pollMessages` (around lines 104-118).

### `src/telegram.zig` — `pollMessages` (lines 108-109)

Current code:

```zig
const offset: usize = @intCast(json.getInt(entity, "offset") orelse continue);
const length: usize = @intCast(json.getInt(entity, "length") orelse continue);
```

Replace with guarded casts that skip entities when either value is negative:

```zig
const raw_offset = json.getInt(entity, "offset") orelse continue;
const raw_length = json.getInt(entity, "length") orelse continue;
if (raw_offset < 0 or raw_length < 0) continue;
const offset: usize = @intCast(raw_offset);
const length: usize = @intCast(raw_length);
```

No other lines change. The subsequent bounds check (`if (offset + length <= text.len and length > 1)`) and mention extraction logic remain as-is.

## Acceptance Criteria

1. **No `@intCast` on unchecked `i64`**: The two `@intCast` calls on the offset and length values must only execute after confirming both values are non-negative.
2. **Negative offset skips entity**: If `json.getInt(entity, "offset")` returns a negative `i64`, the loop iteration executes `continue` without panicking.
3. **Negative length skips entity**: If `json.getInt(entity, "length")` returns a negative `i64`, the loop iteration executes `continue` without panicking.
4. **Valid entities still work**: Entities with non-negative offset and length values are processed exactly as before — the mention-matching logic is unchanged.
5. **Build succeeds**: `zig build` compiles without errors.
6. **Tests pass**: `zig build test` passes with no regressions.

## Edge Cases

1. **Offset is negative, length is valid**: Entity must be skipped entirely (no partial processing).
2. **Length is negative, offset is valid**: Entity must be skipped entirely.
3. **Both offset and length are negative**: Entity must be skipped.
4. **Offset or length is zero**: Zero is a valid `usize` value. An offset of 0 with length of 0 or 1 will be handled by the existing downstream check (`length > 1`), so no special treatment is needed.
5. **Large positive values**: Values that fit in `i64` but exceed `usize` range are not a concern on 64-bit targets (where `usize` is `u64` and all non-negative `i64` values fit). The existing `offset + length <= text.len` check already guards against out-of-bounds slicing.
6. **Missing fields**: Already handled by the `orelse continue` on each `getInt` call — no change needed.
