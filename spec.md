# Spec: Add tests for db.zig proposal lifecycle operations

## 1. Task Summary

`createProposal`, `getProposals`, `updateProposalStatus`, and `getProposal` in
`src/db.zig` implement the autonomous seeding workflow — generating and tracking
improvement proposals — but have zero test coverage. Bugs in these functions
could silently lose proposals, duplicate them, or leave them stuck in an
incorrect status. This task adds a dedicated test file covering the full
lifecycle: create, retrieve by ID, list with and without a status filter, and
all status transitions.

## 2. Files to Modify and Create

| Action | Path |
|--------|------|
| Create | `src/db_proposal_test.zig` |
| Modify | `src/db.zig` — add one line to the existing `test { }` block |

The only change to `src/db.zig` is appending:

```zig
_ = @import("db_proposal_test.zig");
```

inside the existing `test { }` block near the bottom of the file (alongside
the other `_ = @import(...)` lines for `db_pipeline_query_test.zig` and
`db_task_output_test.zig`).

No source functions are added or changed.

## 3. Function / Type Signatures

The functions under test are already present in `src/db.zig`; their signatures
are reproduced here for reference. No signature changes are made.

```zig
// Proposal struct (src/db.zig line 55)
pub const Proposal = struct {
    id: i64,
    repo_path: []const u8,
    title: []const u8,
    description: []const u8,
    rationale: []const u8,
    status: []const u8, // "pending" | "approved" | "dismissed"
    created_at: []const u8,
};

// Insert a new proposal; status defaults to "pending" via DB default.
pub fn createProposal(
    self: *Db,
    repo_path: []const u8,
    title: []const u8,
    description: []const u8,
    rationale: []const u8,
) !i64

// Return up to `limit` proposals, newest first.
// Pass status_filter = null to return all statuses.
pub fn getProposals(
    self: *Db,
    allocator: std.mem.Allocator,
    status_filter: ?[]const u8,
    limit: i64,
) ![]Proposal

// Overwrite the status field of a single proposal row.
pub fn updateProposalStatus(
    self: *Db,
    proposal_id: i64,
    status: []const u8,
) !void

// Return a single proposal by primary key, or null if not found.
pub fn getProposal(
    self: *Db,
    allocator: std.mem.Allocator,
    proposal_id: i64,
) !?Proposal
```

The test file uses the same helper conventions as the existing test files
(`ArenaAllocator` wrapping `std.testing.allocator`, in-memory SQLite via
`Db.init(arena.allocator(), ":memory:")`).

## 4. Acceptance Criteria

Each criterion maps to one or more named tests in `src/db_proposal_test.zig`.

**AC1 — createProposal returns a positive, auto-incremented ID**
- Call `createProposal` once; assert `id > 0`.
- Call it a second time with different args; assert the second `id > first_id`
  (auto-increment strictly increases).

**AC2 — getProposal returns null for a nonexistent ID**
- On an empty DB, `getProposal(arena.allocator(), 9999)` returns `null`.

**AC3 — getProposal returns a correctly populated Proposal**
- After `createProposal("/repo", "T", "D", "R")`, call `getProposal` with the
  returned ID.
- Assert all seven fields:
  - `.id` equals the returned ID.
  - `.repo_path` equals `"/repo"`.
  - `.title` equals `"T"`.
  - `.description` equals `"D"`.
  - `.rationale` equals `"R"`.
  - `.status` equals `"pending"` (DB default, not explicitly written).
  - `.created_at` is non-empty.

**AC4 — New proposals default to "pending" status**
- `createProposal` does not accept a `status` parameter; verify via
  `getProposal` that `.status` is `"pending"` immediately after creation.

**AC5 — getProposals(null, limit) returns all proposals up to limit**
- Insert 3 proposals; call `getProposals(arena.allocator(), null, 10)`.
  Assert `len == 3`.
- Insert 5 proposals; call `getProposals(arena.allocator(), null, 2)`.
  Assert `len == 2` (limit is respected).
- On an empty DB, `getProposals(arena.allocator(), null, 10)` returns a
  slice with `len == 0`.

**AC6 — getProposals with status_filter returns only matching rows**
- Insert 2 `"pending"` and 1 `"approved"` proposal (use
  `updateProposalStatus` to set the latter).
- `getProposals(arena.allocator(), "pending", 10)` returns exactly 2
  entries, all with `.status == "pending"`.
- `getProposals(arena.allocator(), "approved", 10)` returns exactly 1
  entry with `.status == "approved"`.
- `getProposals(arena.allocator(), "dismissed", 10)` returns 0 entries.

**AC7 — getProposals(null, limit) returns proposals newest-first**
- Insert proposals A and B in that order. Because SQLite `datetime('now')`
  has 1-second granularity, force distinct `created_at` values by calling
  `db.sqlite_db.execute("UPDATE proposals SET created_at = ?1 WHERE id = ?2", ...)`.
- Assert that the result slice lists B before A.

**AC8 — updateProposalStatus: pending → approved**
- Create a proposal; call `updateProposalStatus(id, "approved")`.
- `getProposal(id).status` equals `"approved"`.

**AC9 — updateProposalStatus: pending → dismissed**
- Create a proposal; call `updateProposalStatus(id, "dismissed")`.
- `getProposal(id).status` equals `"dismissed"`.

**AC10 — updateProposalStatus does not affect other proposals**
- Insert proposals A and B; update A to `"approved"`.
- `getProposal(B_id).status` is still `"pending"`.

**AC11 — updateProposalStatus is idempotent**
- Call `updateProposalStatus(id, "approved")` twice.
- `getProposal(id).status` equals `"approved"` with no error.

**AC12 — Full lifecycle: create → retrieve → transition → re-retrieve**
- Create a proposal; verify it is `"pending"` via `getProposal`.
- Approve it via `updateProposalStatus`.
- Verify it is `"approved"` via both `getProposal` and
  `getProposals(null, 10)`.
- Dismiss it via `updateProposalStatus`.
- Verify it is `"dismissed"` via `getProposal`.
- Verify it appears in `getProposals("dismissed", 10)` and not in
  `getProposals("pending", 10)` or `getProposals("approved", 10)`.

## 5. Edge Cases

**E1 — Multiple proposals for the same repo_path**
- Two calls to `createProposal("/same/repo", ...)` must return distinct IDs.
- `getProposals(null, 10)` returns both entries.

**E2 — getProposals with limit=0 returns an empty slice**
- Even if proposals exist, `getProposals(arena.allocator(), null, 0)` returns
  a slice with `len == 0`. (The SQL `LIMIT 0` clause produces no rows.)

**E3 — Long description and rationale stored without truncation**
- `createProposal` with a 4 000-character `description` and a 4 000-character
  `rationale`. Verify via `getProposal` that both fields round-trip at full
  length (SQLite TEXT has no length limit).

**E4 — getProposal after deletion returns null**
- Directly execute `DELETE FROM proposals WHERE id = ?1` on the DB to remove a
  known proposal; `getProposal(id)` must return `null` afterward.

**E5 — updateProposalStatus for a nonexistent ID is a no-op (no error)**
- `updateProposalStatus(99999, "approved")` must return without error (the
  `UPDATE … WHERE id = ?` matches zero rows; SQLite treats this as success).

**E6 — getProposals with status_filter matching no rows returns empty slice**
- On a DB with only `"pending"` proposals,
  `getProposals(arena.allocator(), "dismissed", 10)` returns `len == 0`.

**E7 — created_at is DB-generated and non-empty**
- `getProposal` and `getProposals` must always return a non-empty `created_at`
  string; the column is set by `datetime('now')` and is never overridden by
  application code.
