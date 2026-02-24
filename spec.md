# Task #14: Consolidate getPipelineStats into a single SQL query

## Task Summary

`Db.getPipelineStats` in `src/db.zig` (lines 504-520) executes four separate `SELECT COUNT(*)` queries against the `pipeline_tasks` table to compute total, active, merged, and failed counts. Replace these with a single query using conditional `COUNT(CASE WHEN ...)` expressions to reduce database round-trips from four to one and simplify the function body.

## Files to Modify

- `src/db.zig` — Rewrite `getPipelineStats` method body (lines 504-520)

## Files to Create

None.

## Function/Type Signatures

No signature changes. The following remain identical:

```zig
pub const PipelineStats = struct {
    active: i64,
    merged: i64,
    failed: i64,
    total: i64,
};

pub fn getPipelineStats(self: *Db) !PipelineStats
```

### Implementation Detail

Replace the four separate queries:

```sql
SELECT COUNT(*) FROM pipeline_tasks
SELECT COUNT(*) FROM pipeline_tasks WHERE status IN ('backlog', 'spec', 'qa', 'impl', 'retry', 'rebase')
SELECT COUNT(*) FROM pipeline_tasks WHERE status = 'merged'
SELECT COUNT(*) FROM pipeline_tasks WHERE status = 'failed'
```

With a single query:

```sql
SELECT
  COUNT(*) AS total,
  COUNT(CASE WHEN status IN ('backlog', 'spec', 'qa', 'impl', 'retry', 'rebase') THEN 1 END) AS active,
  COUNT(CASE WHEN status = 'merged' THEN 1 END) AS merged,
  COUNT(CASE WHEN status = 'failed' THEN 1 END) AS failed
FROM pipeline_tasks
```

The function body should issue a single `self.sqlite_db.query(self.allocator, ...)` call, extract the four column values from the single result row via `row.getInt(0..3)`, and return the `PipelineStats` struct. Only one `var rows` / `defer rows.deinit()` pair is needed.

## Acceptance Criteria

1. `getPipelineStats` executes exactly **one** SQL query (one call to `self.sqlite_db.query`).
2. The function returns the same `PipelineStats` values as before for all combinations of task statuses.
3. The `PipelineStats` struct is unchanged (fields: `active`, `merged`, `failed`, `total`; all `i64`).
4. The function signature is unchanged: `pub fn getPipelineStats(self: *Db) !PipelineStats`.
5. The caller in `src/web.zig:407` (`self.db.getPipelineStats()`) continues to compile and work without changes.
6. Existing tests pass (`zig build test`), specifically the `"pipeline task lifecycle"` test.
7. A new test `"getPipelineStats returns correct counts"` is added to `src/db.zig` that:
   - Creates an in-memory DB.
   - Inserts tasks with various statuses (backlog, spec, impl, merged, failed, done, rebase, retry, qa).
   - Calls `getPipelineStats()` and asserts `.total`, `.active`, `.merged`, and `.failed` match expected counts.
8. On an empty `pipeline_tasks` table, `getPipelineStats` returns `PipelineStats{ .total = 0, .active = 0, .merged = 0, .failed = 0 }`.

## Edge Cases to Handle

1. **Empty table** — `COUNT(*)` returns 0, and `COUNT(CASE WHEN ... THEN 1 END)` returns 0. The single result row must still be present; the function must handle this correctly (no empty result set — `COUNT` always returns a row).
2. **All tasks in a single status** — e.g., all `merged`: `active` and `failed` should be 0, `merged` and `total` should be equal.
3. **Statuses not covered by active/merged/failed** — Tasks with status `done` are counted in `total` but not in `active`, `merged`, or `failed`. The three conditional counts should not sum to `total` necessarily.
4. **NULL status values** — The schema defaults status to `'backlog'` and the column is `NOT NULL`, so NULL statuses should not occur. However, `CASE WHEN` naturally excludes NULLs (no match), so this is safe regardless.
5. **`getInt` returning null** — If `row.getInt(N)` returns `null` for any column, default to `0` using `orelse 0`, consistent with the existing pattern.
6. **Query failure** — The function returns `!PipelineStats` (error union). If the query fails, the error propagates via `try`, same as before. The caller in `web.zig` already handles this with `catch PipelineStats{ .active = 0, ... }`.
