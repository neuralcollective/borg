# Spec: Add task_id parameter to spawnAgent for container naming

## Task Summary

The `spawnAgent` function in `src/pipeline.zig` has no `task_id` parameter, so container names cannot include the task ID for traceability. Add a `task_id: i64` parameter to `spawnAgent` and update all five call sites (`seedRepo`, `runSpecPhase`, `runQaPhase`, `runImplPhase`, `runRebasePhase`) to pass the appropriate task ID, incorporating it into the Docker container name.

## Files to Modify

1. **`src/pipeline.zig`** — Change `spawnAgent` signature to add `task_id: i64`, update container name format to include the task ID, and update all five call sites.

## Files to Create

None.

## Function/Type Signatures

### `src/pipeline.zig`

#### `spawnAgent` — modify signature (line 1233)

Current:
```zig
fn spawnAgent(self: *Pipeline, persona: AgentPersona, prompt: []const u8, workdir: []const u8, resume_session: ?[]const u8) !agent_mod.AgentResult
```

New:
```zig
fn spawnAgent(self: *Pipeline, task_id: i64, persona: AgentPersona, prompt: []const u8, workdir: []const u8, resume_session: ?[]const u8) !agent_mod.AgentResult
```

#### Container name format — modify (line 1270)

Current:
```zig
const container_name = try std.fmt.bufPrint(&name_buf, "borg-{s}-{d}-{d}", .{
    @tagName(persona), std.time.timestamp(), n,
});
```

New:
```zig
const container_name = try std.fmt.bufPrint(&name_buf, "borg-{s}-t{d}-{d}-{d}", .{
    @tagName(persona), task_id, std.time.timestamp(), n,
});
```

#### Call site updates

1. **`seedRepo`** (line 317) — pass `0` as task_id (no task context):
   ```zig
   self.spawnAgent(0, .manager, prompt_buf.items, repo_path, null)
   ```

2. **`runSpecPhase`** (line 493) — pass `task.id`:
   ```zig
   self.spawnAgent(task.id, .manager, prompt_buf.items, wt_path, null)
   ```

3. **`runQaPhase`** (line 543) — pass `task.id`:
   ```zig
   self.spawnAgent(task.id, .qa, prompt_buf.items, wt_path, resume_sid)
   ```

4. **`runImplPhase`** (line 614) — pass `task.id`:
   ```zig
   self.spawnAgent(task.id, .worker, prompt_buf.items, wt_path, resume_sid)
   ```

5. **`runRebasePhase`** (line 748) — pass `task.id`:
   ```zig
   self.spawnAgent(task.id, .worker, prompt_buf.items, wt_path, resume_sid)
   ```

## Acceptance Criteria

1. **Compiles**: `zig build` succeeds with no errors.
2. **Tests pass**: `zig build test` passes with no regressions.
3. **Signature updated**: `spawnAgent` has a `task_id: i64` parameter as its second argument (after `self`).
4. **Container name includes task ID**: The Docker container name format includes the task ID (e.g. `borg-manager-t19-1700000000-0`).
5. **All call sites updated**: All five call sites pass a task ID — `task.id` for task-scoped phases, `0` for `seedRepo`.
6. **No other behavioral changes**: Agent spawning, Docker config, timeout watchdog, and result parsing remain unchanged.

## Edge Cases

1. **`seedRepo` has no task**: `seedRepo` operates without a `PipelineTask`. Pass `0` as the task_id sentinel value. The container name will read `borg-manager-t0-...`, which is unambiguous since real task IDs start at 1.
2. **Container name length**: Adding `t{task_id}` increases the container name length. With the existing 128-byte `name_buf`, the worst case is roughly `borg-manager-t9999999999-1700000000-4294967295` (46 chars), well within the buffer.
3. **Negative task IDs**: SQLite `ROWID`/`INTEGER PRIMARY KEY` values are always positive (1+), so negative task_id values should not occur in practice. The `i64` type matches the DB schema; no special handling needed.
4. **Log message consistency**: The `std.log.info("Spawning {s} agent: {s}", ...)` at line 1293 already logs the container name, so the task ID will automatically appear in logs via the updated container name — no additional logging changes needed.
