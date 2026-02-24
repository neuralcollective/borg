# Spec: Add tests for db.zig registerGroup and getAllGroups round-trip

## Task Summary

Add comprehensive tests in `src/db.zig` for the `registerGroup`, `getAllGroups`, and `unregisterGroup` functions. The existing round-trip test (line 646) does not verify the `name` or `trigger` fields, and there is zero test coverage for `unregisterGroup`. New tests must verify all five `RegisteredGroup` fields survive a write/read round-trip and that `unregisterGroup` removes groups from subsequent `getAllGroups` results.

## Files to Modify

- `src/db.zig` — append new `test` blocks after the existing tests (after line 855)

## Files to Create

None.

## Function/Type Signatures

No new functions or types are needed. Tests exercise existing public API:

- `pub fn registerGroup(self: *Db, jid: []const u8, name: []const u8, folder: []const u8, trigger: []const u8, requires_trigger: bool) !void`
- `pub fn getAllGroups(self: *Db, allocator: std.mem.Allocator) ![]RegisteredGroup`
- `pub fn unregisterGroup(self: *Db, jid: []const u8) !void`
- `pub fn init(allocator: std.mem.Allocator, path: [:0]const u8) !Db`
- `pub fn deinit(self: *Db) void`

Each new test block uses the standard pattern:
```zig
test "descriptive name" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();
    // ... assertions ...
}
```

## Acceptance Criteria

### AC1: Round-trip preserves all five fields
- Register a group with known values for all five fields: `jid="grp:rt1"`, `name="Round Trip Group"`, `folder="rt-folder"`, `trigger="@TestBot"`, `requires_trigger=true`.
- Call `getAllGroups` and assert exactly one result.
- Assert `group.jid` equals `"grp:rt1"`.
- Assert `group.name` equals `"Round Trip Group"`.
- Assert `group.folder` equals `"rt-folder"`.
- Assert `group.trigger` equals `"@TestBot"`.
- Assert `group.requires_trigger` equals `true`.

### AC2: Round-trip with requires_trigger=false
- Register a group with `requires_trigger=false`.
- Call `getAllGroups` and assert the returned group has `requires_trigger == false`.
- Assert all other fields match their input values.

### AC3: Round-trip with custom trigger pattern
- Register a group with a non-default trigger (e.g., `"!cmd"`).
- Call `getAllGroups` and assert `group.trigger` equals `"!cmd"`.

### AC4: unregisterGroup removes group from getAllGroups
- Register two groups with distinct JIDs.
- Call `getAllGroups` and assert count is 2.
- Call `unregisterGroup` with the first group's JID.
- Call `getAllGroups` and assert count is 1.
- Assert the remaining group's JID matches the second group's JID.

### AC5: unregisterGroup on non-existent JID does not error
- Initialize a fresh in-memory DB.
- Call `unregisterGroup("nonexistent:jid")` — must not return an error.
- Call `getAllGroups` and assert count is 0.

### AC6: getAllGroups on empty database returns empty slice
- Initialize a fresh in-memory DB (no groups registered).
- Call `getAllGroups` and assert the returned slice has length 0.

### AC7: Register, unregister, re-register round-trip
- Register a group with JID `"grp:cycle"`.
- Unregister it.
- Re-register with the same JID but different `name` and `trigger`.
- Call `getAllGroups` and assert count is 1.
- Assert the returned group has the new `name` and `trigger` values (not the originals).

### AC8: All tests pass
- `zig build test` completes with zero failures.

## Edge Cases to Handle

1. **Empty string fields**: Register a group where `trigger` is `""` (empty string). Verify `getAllGroups` returns `""` for the trigger field, not the schema default `"@Borg"`. Note: the current `getAllGroups` implementation uses `row.get(3) orelse "@Borg"` — an empty string from SQLite is non-null, so it should be preserved. The test must confirm this.

2. **Unicode in name/trigger**: Register a group with Unicode characters in `name` (e.g., `"Группа Тест"`) and `trigger` (e.g., `"@Бот"`). Verify round-trip preserves the exact byte sequences.

3. **Unregister middle of multiple groups**: Register three groups (A, B, C). Unregister B. Assert `getAllGroups` returns exactly A and C with all fields intact.

4. **Double unregister**: Call `unregisterGroup` twice for the same JID. The second call must not error (DELETE on non-existent row is a no-op in SQLite).

5. **Upsert via registerGroup preserves only latest values**: Register with JID `"grp:upsert"` and `name="V1"`. Register again with same JID and `name="V2"`. The existing test at line 665 covers this partially, but confirm `trigger` and `folder` fields also update correctly on upsert.
