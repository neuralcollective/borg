# Spec: Fix SSE client file descriptor leak in web server

## Task Summary

In `src/web.zig`, when SSE client writes fail during broadcast, the client's `std.net.Stream` is removed from the tracking list via `swapRemove()` but never closed, leaking the underlying file descriptor. Since `handleConnection()` deliberately skips closing the stream for SSE paths (returning before the `stream.close()` call on line 229), disconnected SSE clients accumulate leaked file descriptors indefinitely. The fix must call `stream.close()` on every removed SSE client entry across all three broadcast sites, and also close all remaining SSE client streams during server shutdown.

## Files to Modify

- `src/web.zig`

## Files to Create

_(none)_

## Function/Type Signatures

No new functions or types are needed. The following existing functions require changes:

### `fn broadcastSse(self: *WebServer, level: []const u8, message: []const u8) void`
**Location:** `src/web.zig:126`
**Change:** Before the `continue` in the `catch` block (line 143), close the stream that was swap-removed. Since `swapRemove(i)` returns the removed element, capture it and call `.close()` on it.

```zig
// Current (line 142-144):
self.sse_clients.items[i].writeAll(line) catch {
    _ = self.sse_clients.swapRemove(i);
    continue;
};

// New:
self.sse_clients.items[i].writeAll(line) catch {
    const removed = self.sse_clients.swapRemove(i);
    removed.close();
    continue;
};
```

### `fn broadcastChatEvent(self: *WebServer, text: []const u8) void`
**Location:** `src/web.zig:103`
**Change:** Same pattern — close the removed stream in the `catch` block (line 118-120).

```zig
// Current (line 118-120):
self.chat_sse_clients.items[i].writeAll(line) catch {
    _ = self.chat_sse_clients.swapRemove(i);
    continue;
};

// New:
self.chat_sse_clients.items[i].writeAll(line) catch {
    const removed = self.chat_sse_clients.swapRemove(i);
    removed.close();
    continue;
};
```

### `fn handleChatPost(self: *WebServer, stream: std.net.Stream, request: []const u8) void`
**Location:** `src/web.zig:313`
**Change:** Same pattern in the inline chat SSE broadcast loop (line 371-373).

```zig
// Current (line 371-373):
self.chat_sse_clients.items[i].writeAll(line) catch {
    _ = self.chat_sse_clients.swapRemove(i);
    continue;
};

// New:
self.chat_sse_clients.items[i].writeAll(line) catch {
    const removed = self.chat_sse_clients.swapRemove(i);
    removed.close();
    continue;
};
```

### `pub fn stop(self: *WebServer) void`
**Location:** `src/web.zig:177`
**Change:** After setting `running` to false and before the self-connect unblock, close all remaining SSE client streams in both `sse_clients` and `chat_sse_clients` to prevent leaking file descriptors on shutdown.

```zig
// Add after line 178 (self.running.store(false, .release)):
{
    self.sse_mu.lock();
    defer self.sse_mu.unlock();
    for (self.sse_clients.items) |client| {
        client.close();
    }
    self.sse_clients.clearRetainingCapacity();
}
{
    self.chat_sse_mu.lock();
    defer self.chat_sse_mu.unlock();
    for (self.chat_sse_clients.items) |client| {
        client.close();
    }
    self.chat_sse_clients.clearRetainingCapacity();
}
```

## Acceptance Criteria

1. **`broadcastSse` closes removed streams:** When `writeAll` fails for an SSE log client, `stream.close()` is called on the removed entry before continuing the loop.
2. **`broadcastChatEvent` closes removed streams:** When `writeAll` fails for a chat SSE client, `stream.close()` is called on the removed entry before continuing the loop.
3. **`handleChatPost` closes removed streams:** When `writeAll` fails for a chat SSE client in the inline broadcast loop, `stream.close()` is called on the removed entry before continuing the loop.
4. **`stop()` closes all remaining SSE clients:** On server shutdown, all streams in `sse_clients` and `chat_sse_clients` are closed before the lists are cleared.
5. **Existing tests pass:** `zig build test` passes without regressions. The `jsonEscape` and `parsePath` tests in `src/web.zig` continue to pass.
6. **No double-close:** The `swapRemove()` return value is used exactly once for `.close()`, ensuring no stream is closed twice.

## Edge Cases

1. **`stream.close()` itself fails:** `std.net.Stream.close()` returns `void` in Zig's standard library (it calls `std.posix.close` which cannot fail in practice for valid fds), so no error handling is needed.
2. **Empty client list:** The broadcast loops already handle empty lists correctly (the `while` condition is false immediately). No change needed.
3. **All clients fail simultaneously:** The `swapRemove` + close pattern works correctly even when every client in the list fails, because `swapRemove` shrinks the list and the loop index `i` is not incremented on removal.
4. **Concurrent access during shutdown:** The `stop()` cleanup acquires `sse_mu` and `chat_sse_mu` before iterating, which is consistent with the locking discipline used by broadcast functions. A broadcast in progress will complete before `stop()` acquires the lock (or vice versa), preventing double-close races.
5. **SSE client disconnects between registration and first broadcast:** The client's first failed write will trigger removal + close. This is the normal path and works correctly with the fix.
6. **Server accepts new SSE connections while `stop()` is running:** After `stop()` clears the lists, the `run()` loop may still accept one more connection (the self-connect used to unblock `accept()`). This is not an SSE connection, so it is handled normally and closed by `handleConnection`.
