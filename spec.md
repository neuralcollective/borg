# Spec: Add tests for db.zig registerGroup/getAllGroups/unregisterGroup

## Task Summary

The existing "group registration round trip" test in `src/db.zig` (line 671) verifies `registerGroup` and `getAllGroups` but does not assert all fields — it skips checking `name` and `trigger`. The `unregisterGroup` function (line 255) has zero test coverage. Add tests that verify all five `RegisteredGroup` fields survive a register/read round-trip, and that `unregisterGroup` removes the group from subsequent `getAllGroups` results.

## Files to Modify

1. **`src/db.zig`** — Add new `test` blocks in the existing test section at the bottom of the file (after line 881).

## Files to Create

None.

## Function/Type Signatures

No new functions or types. The tests exercise existing public API:

```zig
// src/db.zig — existing signatures (no changes)
pub fn registerGroup(self: *Db, jid: []const u8, name: []const u8, folder: []const u8, trigger: []const u8, requires_trigger: bool) !void
pub fn getAllGroups(self: *Db, allocator: std.mem.Allocator) ![]RegisteredGroup
pub fn unregisterGroup(self: *Db, jid: []const u8) !void
```

```zig
// src/db.zig — existing struct (no changes)
pub const RegisteredGroup = struct {
    jid: []const u8,
    name: []const u8,
    folder: []const u8,
    trigger: []const u8,
    requires_trigger: bool,
};
```

### New test blocks to add

1. **`test "registerGroup and getAllGroups round-trip preserves all fields"`** — Registers a single group with non-default values for every field (including a custom trigger like `"!help"` and `requires_trigger = false`), calls `getAllGroups`, and asserts all five fields (`jid`, `name`, `folder`, `trigger`, `requires_trigger`) match exactly.

2. **`test "unregisterGroup removes group from getAllGroups"`** — Registers two groups, calls `unregisterGroup` on the first, then calls `getAllGroups` and asserts only the second group remains (count == 1) and its `jid` matches the non-removed group.

3. **`test "unregisterGroup on nonexistent jid is a no-op"`** — Opens a fresh in-memory DB, calls `unregisterGroup` with a jid that was never registered, and asserts no error is returned (DELETE WHERE on a missing row is not an error in SQLite).

4. **`test "getAllGroups returns empty slice when no groups registered"`** — Opens a fresh in-memory DB with no registrations, calls `getAllGroups`, and asserts the result has length 0.

## Acceptance Criteria

1. **All-fields round-trip**: Registering a group with `jid="g1"`, `name="Test Group"`, `folder="test-folder"`, `trigger="!help"`, `requires_trigger=false` and reading it back via `getAllGroups` returns exactly one `RegisteredGroup` where `jid`, `name`, `folder`, `trigger`, and `requires_trigger` all match the input values.

2. **Unregister removes group**: After registering groups with jids `"g1"` and `"g2"`, calling `unregisterGroup("g1")` causes `getAllGroups` to return exactly one group with `jid == "g2"`.

3. **Unregister nonexistent is safe**: Calling `unregisterGroup("nonexistent")` on a DB with no matching row does not return an error.

4. **Empty result**: `getAllGroups` on a fresh in-memory DB (no prior `registerGroup` calls) returns a slice of length 0.

5. **Tests compile and pass**: `zig build test` passes with all new tests included.

## Edge Cases

1. **Custom trigger pattern**: The `trigger_pattern` column has a default of `'@Borg'` in the schema. The round-trip test must use a non-default trigger (e.g., `"!help"`) to prove the value comes from the insert, not the column default.

2. **`requires_trigger = false`**: The column default is `1` (true). The round-trip test must use `false` to verify the boolean is stored and read correctly, not just falling back to the default.

3. **Unregister nonexistent jid**: SQLite `DELETE WHERE jid = ?` on a non-matching row returns success (0 rows affected). The test confirms `unregisterGroup` does not propagate an error in this case.

4. **Order independence**: `getAllGroups` does not guarantee row ordering (no ORDER BY). Tests that check a specific group after unregister should either register only two groups (so after removal there's exactly one) or search the returned slice by jid rather than assuming positional order.

5. **`INSERT OR REPLACE` semantics**: The existing "registerGroup upserts on conflict" test already covers this case (re-registering the same jid overwrites all fields). The new tests should not duplicate this; they focus on round-trip field fidelity and unregister behavior.
