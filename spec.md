# Spec: Extract `rowToQueueEntry` helper in db.zig

## 1. Task Summary

`getQueuedBranches` and `getQueuedBranchesForRepo` in `src/db.zig` contain
identical seven-field row-to-`QueueEntry` mapping blocks (lines 629–637 and
652–660). Extracting a private `rowToQueueEntry` helper eliminates the
duplication, mirrors the existing `rowToPipelineTask` pattern already present
in the file, and guarantees both functions stay in sync if the
`integration_queue` schema or `QueueEntry` struct changes.

## 2. Files to Modify and Create

| Action | Path |
|--------|------|
| Modify | `src/db.zig` |

No new files are created.

## 3. Function / Type Signatures

### New helper (private, file-scope, matching the `rowToPipelineTask` pattern)

```zig
fn rowToQueueEntry(allocator: std.mem.Allocator, row: sqlite.Row) !QueueEntry
```

- `allocator` — used for `dupe` calls on string fields.
- `row` — a `sqlite.Row` value produced by `sqlite_db.query`; column order
  must match the SELECT projection used in both callers:
  `id(0), task_id(1), branch(2), repo_path(3), status(4), queued_at(5), pr_number(6)`.
- Returns `!QueueEntry`; propagates allocator errors with `try`.

Field mapping inside the helper (must match the current inline blocks exactly):

| Index | Field | Expression |
|-------|-------|------------|
| 0 | `.id` | `row.getInt(0) orelse 0` |
| 1 | `.task_id` | `row.getInt(1) orelse 0` |
| 2 | `.branch` | `try allocator.dupe(u8, row.get(2) orelse "")` |
| 3 | `.repo_path` | `try allocator.dupe(u8, row.get(3) orelse "")` |
| 4 | `.status` | `try allocator.dupe(u8, row.get(4) orelse "queued")` |
| 5 | `.queued_at` | `try allocator.dupe(u8, row.get(5) orelse "")` |
| 6 | `.pr_number` | `row.getInt(6) orelse 0` |

### Changed callers (public signatures unchanged)

```zig
pub fn getQueuedBranches(self: *Db, allocator: std.mem.Allocator) ![]QueueEntry
pub fn getQueuedBranchesForRepo(self: *Db, allocator: std.mem.Allocator, repo_path: []const u8) ![]QueueEntry
```

Both functions replace their inline `QueueEntry{ … }` struct literals with:

```zig
try entries.append(try rowToQueueEntry(allocator, row));
```

## 4. Acceptance Criteria

1. `just b` compiles `src/db.zig` without errors or warnings.
2. `just t` passes all existing tests, including the integration-queue tests
   that exercise `getQueuedBranches` and `getQueuedBranchesForRepo`.
3. The inline column-index mapping block (`QueueEntry{ .id = row.getInt(0) … }`)
   appears **exactly once** in `src/db.zig` — inside `rowToQueueEntry` — and
   **not** inside `getQueuedBranches` or `getQueuedBranchesForRepo`.
4. `rowToQueueEntry` is **not** `pub`; it is a private helper (consistent with
   `rowToPipelineTask`).
5. The helper is placed adjacent to the two callers (within the
   integration-queue section of `src/db.zig`, after `enqueueForIntegration`
   and before or immediately after `getQueuedBranches`).
6. No public API surfaces change: `QueueEntry`, `getQueuedBranches`, and
   `getQueuedBranchesForRepo` retain their existing signatures and semantics.

## 5. Edge Cases

- **Null columns at runtime**: `COALESCE` in both SQL queries already converts
  `NULL` `repo_path` and `pr_number` at the DB layer. The helper still applies
  `orelse` defaults (`""`, `0`, `"queued"`) for safety against any future query
  variation that might omit the `COALESCE`.
- **Allocator failure mid-struct**: if `dupe` fails on e.g. `branch`, later
  fields are never allocated. The error propagates naturally; the partially
  built `entries` ArrayList is freed by the caller (the `defer` on `rows` does
  not free `QueueEntry` string fields, so callers that have error-path cleanup
  must handle this — no change from the current behaviour).
- **Empty result sets**: when no rows match, the helper is never called; both
  callers already return an empty slice without error. No change needed.
- **Column order change**: the helper centralises column-index assumptions;
  updating them in one place is the primary motivation for this refactor.
- **No behavioural change**: this is a pure refactor — no SQL queries, public
  APIs, struct definitions, or test logic are altered.
