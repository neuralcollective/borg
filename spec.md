# Spec: Consolidate getPipelineStats into a single SQL query

## Task Summary

`src/db.zig:getPipelineStats` (lines 520-536) executes four separate `SELECT COUNT(*)` queries against the `pipeline_tasks` table to obtain total, active, merged, and failed counts. Replace these with a single query using conditional `COUNT(CASE WHEN ...)` expressions, reducing database round-trips from four to one and simplifying the function body from four query/defer/extract blocks to one.

## Files to Modify

1. **`src/db.zig`** — Rewrite `getPipelineStats` (lines 520-536) to use a single SQL query with conditional aggregation.

## Files to Create

None.

## Function/Type Signatures

### `src/db.zig`

#### `Db.PipelineStats` — unchanged (lines 513-518)

```zig
pub const PipelineStats = struct {
    active: i64,
    merged: i64,
    failed: i64,
    total: i64,
};
```

No changes to the struct definition.

#### `Db.getPipelineStats` — modify (lines 520-536)

```zig
pub fn getPipelineStats(self: *Db) !PipelineStats
```

Signature is unchanged. The body should execute a single query:

```sql
SELECT
  COUNT(*) AS total,
  COUNT(CASE WHEN status IN ('backlog', 'spec', 'qa', 'impl', 'retry', 'rebase') THEN 1 END) AS active,
  COUNT(CASE WHEN status = 'merged' THEN 1 END) AS merged,
  COUNT(CASE WHEN status = 'failed' THEN 1 END) AS failed
FROM pipeline_tasks
```

The result row has four columns (indices 0-3). Extract each with `row.getInt(N) orelse 0`. The function should declare a single `var rows`, `defer rows.deinit()`, and return the populated `PipelineStats` struct.

## Acceptance Criteria

1. **Single query**: `getPipelineStats` executes exactly one SQL statement against `pipeline_tasks`, not four.
2. **Same return values**: For any database state, the returned `PipelineStats` values (`.total`, `.active`, `.merged`, `.failed`) are identical to those produced by the original four-query implementation.
3. **Active status list matches**: The `IN (...)` clause for active tasks must use the same six statuses as the original: `'backlog', 'spec', 'qa', 'impl', 'retry', 'rebase'`. This must also match the list used in `getNextPipelineTask` (line 371) and `getActivePipelineTaskCount` (line 541).
4. **Build succeeds**: `zig build` compiles without errors.
5. **Tests pass**: `zig build test` passes. The existing `"pipeline task lifecycle"` test (line 791) and all other tests continue to pass without modification.
6. **No public API change**: `Db.PipelineStats` struct and `Db.getPipelineStats` signature remain unchanged. The caller in `src/web.zig:672` requires no modification.
7. **No extra allocations**: The function continues to use `self.allocator` for the query rows (same as before) and does not introduce any new heap allocations beyond what `sqlite_db.query` already does.

## Edge Cases

1. **Empty table**: When `pipeline_tasks` has zero rows, all four counts must return 0 (not null). `COUNT(*)` returns 0 on empty tables, and `COUNT(CASE WHEN ... THEN 1 END)` also returns 0 (not null) when no rows match, so `getInt` will parse "0" correctly.
2. **All tasks in one status**: If every task is e.g. `'backlog'`, then `active` equals `total`, and `merged` and `failed` are both 0.
3. **Statuses not in any category**: Tasks with status `'done'`, `'test'`, or other statuses not listed in the active/merged/failed conditions contribute only to `total`. The sum of `active + merged + failed` may be less than `total` — this is correct and matches the original behavior.
4. **Query failure**: If the single query fails, the function returns an error (via `try`), same as the original. The caller in `src/web.zig:672` already catches errors and falls back to a zeroed `PipelineStats`.
5. **Row parsing with no columns**: If `rows.items` is empty (should not happen for an aggregate query, but defensively), the function should return all zeros, same as the original behavior with the `if (rows.items.len > 0)` guards.
