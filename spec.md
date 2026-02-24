# Spec: Fix is_bot_message column always mirroring is_from_me in storeMessage

## Task Summary

In `src/db.zig:190-204`, the `storeMessage` function binds both the `is_from_me` and `is_bot_message` SQL parameters to `msg.is_from_me`, making the `is_bot_message` column a redundant copy of `is_from_me`. A dedicated `is_bot_message` field must be added to the `Message` struct and bound independently in the SQL INSERT. All call sites in `main.zig` and test files must be updated to set `is_bot_message = true` for bot-generated responses and `false` for user messages.

## Files to Modify

1. `src/db.zig` — Add `is_bot_message` field to `Message` struct; bind it independently in `storeMessage`; read it back in `getMessagesSince`; update existing tests.
2. `src/main.zig` — Set `is_bot_message` correctly at each `storeMessage` call site and in test data.

## Files to Create

None.

## Function/Type Signature Changes

### `src/db.zig`

#### `Message` struct (line 12-20)

Add `is_bot_message: bool` field:

```zig
pub const Message = struct {
    id: []const u8,
    chat_jid: []const u8,
    sender: []const u8,
    sender_name: []const u8,
    content: []const u8,
    timestamp: []const u8,
    is_from_me: bool,
    is_bot_message: bool,  // NEW
};
```

#### `storeMessage` (line 190-204)

Change the 8th bind parameter (line 201) from `msg.is_from_me` to `msg.is_bot_message`:

```zig
@as(i64, if (msg.is_bot_message) 1 else 0),  // was: msg.is_from_me
```

No signature change to `storeMessage` itself — it already takes `msg: Message`.

#### `getMessagesSince` (line 206-227)

Update the SELECT query (line 209) to also retrieve `is_bot_message`:

```sql
SELECT id, chat_jid, sender, sender_name, content, timestamp, is_from_me, is_bot_message
FROM messages WHERE chat_jid = ?1 AND timestamp > ?2 ORDER BY timestamp ASC LIMIT 50
```

Update the `Message` construction (line 216-224) to populate the new field:

```zig
.is_bot_message = (row.getInt(7) orelse 0) == 1,
```

### `src/main.zig`

#### Incoming user message storage (line 594-602)

Add `.is_bot_message = false` to the `Message` literal:

```zig
db.storeMessage(.{
    ...
    .is_from_me = false,
    .is_bot_message = false,  // NEW
}) catch return;
```

#### Bot response storage (line 825-833)

Add `.is_bot_message = true` to the `Message` literal:

```zig
db.storeMessage(.{
    ...
    .is_from_me = true,
    .is_bot_message = true,  // NEW
}) catch {};
```

#### Test data in `formatPrompt` test (lines 1186-1188)

Add `.is_bot_message` to each test `Message` literal:

```zig
.{ ..., .is_from_me = false, .is_bot_message = false },
.{ ..., .is_from_me = true, .is_bot_message = true },
```

### `src/db.zig` — Tests

All existing `storeMessage` test calls (lines 474-476, 502-503) must add `.is_bot_message = false`.

## Acceptance Criteria

1. **Struct field exists**: `db_mod.Message` has a field `is_bot_message` of type `bool`.
2. **Independent SQL binding**: In `storeMessage`, the `is_bot_message` column is bound to `msg.is_bot_message`, not `msg.is_from_me`. Storing a `Message` with `is_from_me = true, is_bot_message = false` must write `is_from_me=1, is_bot_message=0` to the database.
3. **Round-trip read**: `getMessagesSince` returns `Message` structs with `is_bot_message` populated from the database column.
4. **User messages stored correctly**: When `main.zig` stores an incoming user message (the call site at ~line 594), `is_from_me = false` and `is_bot_message = false`.
5. **Bot responses stored correctly**: When `main.zig` stores a bot response (the call site at ~line 825), `is_from_me = true` and `is_bot_message = true`.
6. **All existing tests pass**: `zig build test` succeeds with no regressions.
7. **New test**: A unit test in `db.zig` stores a message with `is_from_me = true, is_bot_message = false` and another with `is_from_me = false, is_bot_message = true`, then reads them back and asserts the two fields are independent.

## Edge Cases to Handle

1. **Existing database rows**: The `is_bot_message` column already exists in the schema with `DEFAULT 0`. Existing rows that were inserted before this fix will have `is_bot_message = 0`. No migration is needed for the schema, but historical data will be inaccurate (all old bot messages will show `is_bot_message = 0` even if they were from the bot). This is acceptable — the column was always incorrectly populated.
2. **Message deduplication**: `INSERT OR IGNORE` means if a message already exists (same `chat_jid` + `id`), the new `is_bot_message` value is silently dropped. This is existing behavior and does not change.
3. **is_from_me = true but is_bot_message = false**: This combination is valid (e.g., the human user sending from the same account the bot runs on). The code must not conflate the two fields.
4. **is_from_me = false but is_bot_message = true**: This combination is theoretically possible (a bot message relayed through a different sender identity). The struct should allow it even if no current call site produces it.
5. **All anonymous struct literals**: Every place that constructs a `Message` using `.{ ... }` syntax must include `.is_bot_message`, otherwise the Zig compiler will emit a missing-field error. All such sites are enumerated above.
