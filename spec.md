# Task #8: Fix Docker Container Name Collision for Concurrent Agents

## 1. Task Summary

Container names in `Pipeline.spawnAgent` are generated using `std.time.timestamp()` (second granularity), which causes Docker to reject a second container creation if two agents are spawned within the same second. A function-local anonymous-struct atomic counter (`const seq = struct { var counter = ...; };`) was added as a partial fix, but the counter is an implicit file-scoped static rather than an explicit `Pipeline` field, making ownership and initialization non-obvious. The clean fix is to promote the counter to an explicit `container_seq: std.atomic.Value(u32)` field on `Pipeline` and drop the redundant timestamp from the name format.

## 2. Files to Modify

| File | Reason |
|------|--------|
| `src/pipeline.zig` | Add `container_seq` field to `Pipeline` struct, initialize it in `init`, update `spawnAgent` to use it, remove function-local `seq` anonymous struct and `std.time.timestamp()` from the name format. |

## 3. Files to Create

None.

## 4. Function/Type Signatures for New or Changed Code

### `Pipeline` struct — add field

```zig
pub const Pipeline = struct {
    // ... existing fields ...
    active_agents: std.atomic.Value(u32),
    container_seq: std.atomic.Value(u32),  // monotonic counter for unique container names
};
```

### `Pipeline.init` — initialize new field

```zig
return .{
    // ... existing fields ...
    .active_agents = std.atomic.Value(u32).init(0),
    .container_seq = std.atomic.Value(u32).init(0),
};
```

### `Pipeline.spawnAgent` — replace function-local counter with struct field

Remove the current block (lines ~1362–1370):

```zig
var name_buf: [128]u8 = undefined;
const seq = struct {
    var counter = std.atomic.Value(u32).init(0);
};
const n = seq.counter.fetchAdd(1, .monotonic);
const container_name = try std.fmt.bufPrint(&name_buf, "borg-{s}-{d}-{d}", .{
    @tagName(persona), std.time.timestamp(), n,
});
```

Replace with:

```zig
var name_buf: [128]u8 = undefined;
const n = self.container_seq.fetchAdd(1, .monotonic);
const container_name = try std.fmt.bufPrint(&name_buf, "borg-{s}-{d}", .{
    @tagName(persona), n,
});
```

## 5. Acceptance Criteria

1. `Pipeline` struct declares `container_seq: std.atomic.Value(u32)`.
2. `Pipeline.init` initializes `container_seq` to `0`.
3. `spawnAgent` uses `self.container_seq.fetchAdd(1, .monotonic)` — no function-local `seq` struct remains.
4. `std.time.timestamp()` is no longer part of the container name format string inside `spawnAgent`.
5. Generated container names match the pattern `borg-{persona}-{n}` where `n` is the u32 counter value.
6. Two successive calls to the name-generation logic within the same second produce distinct names (counter increments from `n` to `n+1`).
7. A unit test asserts that two container name strings produced by incrementing `container_seq` are not equal.
8. `just t` passes with no regressions.

## 6. Edge Cases to Handle

| Edge case | Expected behaviour |
|-----------|-------------------|
| Counter wrap-around at `u32` max (4 294 967 295) | `fetchAdd` wraps to 0; names remain unique in practice within a process lifetime (no realistic workload spawns 4B containers). No special handling needed; document the wrap in a code comment. |
| Multiple `Pipeline` instances in tests | Each instance starts its own counter at 0; names are unique per instance. This is acceptable since production runs exactly one `Pipeline`. |
| `name_buf` overflow | `"borg-worker-4294967295"` is 22 bytes, well within the 128-byte buffer. A `comptime` assertion or comment confirming buffer adequacy should be added. |
| Watchdog thread uses container name | `name_for_watchdog` is `allocator.dupe(u8, container_name)` taken after the name is formatted; no change to the watchdog code path is required. |
| Concurrent `spawnAgent` calls | `fetchAdd(.monotonic)` is atomic; two simultaneous callers always receive distinct values. |
