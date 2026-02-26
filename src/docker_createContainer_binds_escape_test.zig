// Tests for Task #80: JSON-escape bind mount paths in Docker createContainer API body
//
// Covers every acceptance criterion from spec.md:
//
//   AC1 — Normal bind paths pass through unchanged in the JSON body.
//   AC2 — A double-quote (`"`) inside a bind path is JSON-escaped to `\"`.
//   AC3 — A backslash (`\`) inside a bind path is JSON-escaped to `\\`.
//   AC4 — Control characters (newline, tab) are JSON-escaped to `\n`, `\t`, etc.
//   AC5 — Multiple binds are comma-separated and each is independently escaped.
//   AC6 — An empty binds slice produces `"Binds":[]` in the body.
//   AC7 — Env-var escaping (the existing loop) is not broken by the change.
//
// Edge cases:
//   E1  — A literal two-character sequence backslash-n (not a newline byte) is
//          escaped as `\\n` (backslash becomes `\\`, then `n` passes through).
//   E2  — A bind string whose only special character is a double-quote and it is
//          also the only element; the comma-skip logic for i==0 must still work.
//   E3  — Unicode / multi-byte UTF-8 sequences pass through unchanged (no double
//          escaping).
//
// How to include in the build:
//   Inside docker.zig's trailing test block add:
//       _ = @import("docker_createContainer_binds_escape_test.zig");
//
// These tests FAIL to compile until `docker.buildContainerBody` is added as a
// public function to docker.zig.  Once the function exists AND the escaping fix
// is applied the tests pass.

const std = @import("std");
const docker = @import("docker.zig");

// Helper: build the JSON body for a ContainerConfig and return it as a slice.
// Callers must free with alloc.free().
fn buildBody(alloc: std.mem.Allocator, config: docker.ContainerConfig) ![]u8 {
    // buildContainerBody does not exist yet → compile error → tests fail.
    return docker.buildContainerBody(alloc, config);
}

// =============================================================================
// AC1: Normal bind paths pass through verbatim
// =============================================================================

test "AC1: plain bind path appears verbatim in JSON body" {
    const alloc = std.testing.allocator;
    const binds = [_][]const u8{"/home/user/project:/workspace"};
    const config = docker.ContainerConfig{
        .image = "test:latest",
        .name = "test-c",
        .env = &.{},
        .binds = &binds,
    };
    const body = try buildBody(alloc, config);
    defer alloc.free(body);

    // The bind string must appear verbatim between JSON quotes.
    try std.testing.expect(std.mem.indexOf(u8, body, "\"/home/user/project:/workspace\"") != null);
}

test "AC1: bind path with colons and rw option appears verbatim" {
    const alloc = std.testing.allocator;
    const binds = [_][]const u8{"/tmp/data:/data:rw"};
    const config = docker.ContainerConfig{
        .image = "img",
        .name = "c",
        .env = &.{},
        .binds = &binds,
    };
    const body = try buildBody(alloc, config);
    defer alloc.free(body);

    try std.testing.expect(std.mem.indexOf(u8, body, "\"/tmp/data:/data:rw\"") != null);
}

// =============================================================================
// AC2: Double-quote in a bind path is JSON-escaped to \"
// =============================================================================

test "AC2: double-quote in bind path is escaped to backslash-quote" {
    const alloc = std.testing.allocator;
    // Bind string: /tmp/dir"with"quotes:/dst
    const bind_str = "/tmp/dir\"with\"quotes:/dst";
    const binds = [_][]const u8{bind_str};
    const config = docker.ContainerConfig{
        .image = "img",
        .name = "c",
        .env = &.{},
        .binds = &binds,
    };
    const body = try buildBody(alloc, config);
    defer alloc.free(body);

    // The raw double-quote must NOT appear unescaped between the enclosing quotes.
    // The escaped form is \" so we expect \\\" in the raw body bytes.
    try std.testing.expect(std.mem.indexOf(u8, body, "\\\"with\\\"") != null);
    // Confirm the overall JSON is valid by checking the Binds array wrapper.
    try std.testing.expect(std.mem.indexOf(u8, body, "\"Binds\":[") != null);
}

test "AC2: body remains valid JSON when bind path contains a double-quote" {
    const alloc = std.testing.allocator;
    const bind_str = "/path/to/\"quoted\":/dst";
    const binds = [_][]const u8{bind_str};
    const config = docker.ContainerConfig{
        .image = "img",
        .name = "c",
        .env = &.{},
        .binds = &binds,
    };
    const body = try buildBody(alloc, config);
    defer alloc.free(body);

    // Validate via std.json: the body must parse without error.
    var parsed = try std.json.parseFromSlice(std.json.Value, alloc, body, .{});
    defer parsed.deinit();

    // Confirm the parsed HostConfig.Binds[0] round-trips to the original string.
    const host_config = parsed.value.object.get("HostConfig") orelse
        return error.MissingHostConfig;
    const binds_arr = host_config.object.get("Binds") orelse
        return error.MissingBinds;
    try std.testing.expectEqual(@as(usize, 1), binds_arr.array.items.len);
    try std.testing.expectEqualStrings(bind_str, binds_arr.array.items[0].string);
}

// =============================================================================
// AC3: Backslash in a bind path is JSON-escaped to \\
// =============================================================================

test "AC3: backslash in bind path is escaped to double-backslash" {
    const alloc = std.testing.allocator;
    // Bind string: /tmp/dir\sub:/dst   (one backslash in host path)
    const bind_str = "/tmp/dir\\sub:/dst";
    const binds = [_][]const u8{bind_str};
    const config = docker.ContainerConfig{
        .image = "img",
        .name = "c",
        .env = &.{},
        .binds = &binds,
    };
    const body = try buildBody(alloc, config);
    defer alloc.free(body);

    // In the raw body, the one backslash should appear as \\
    try std.testing.expect(std.mem.indexOf(u8, body, "dir\\\\sub") != null);
}

test "AC3: body is valid JSON when bind path contains a backslash" {
    const alloc = std.testing.allocator;
    const bind_str = "/tmp/dir\\sub:/dst";
    const binds = [_][]const u8{bind_str};
    const config = docker.ContainerConfig{
        .image = "img",
        .name = "c",
        .env = &.{},
        .binds = &binds,
    };
    const body = try buildBody(alloc, config);
    defer alloc.free(body);

    var parsed = try std.json.parseFromSlice(std.json.Value, alloc, body, .{});
    defer parsed.deinit();

    const host_config = parsed.value.object.get("HostConfig") orelse
        return error.MissingHostConfig;
    const binds_arr = host_config.object.get("Binds") orelse
        return error.MissingBinds;
    try std.testing.expectEqualStrings(bind_str, binds_arr.array.items[0].string);
}

// =============================================================================
// AC4: Control characters in a bind path are JSON-escaped
// =============================================================================

test "AC4: newline byte in bind path is escaped to \\n" {
    const alloc = std.testing.allocator;
    // Bind string containing a literal newline byte in the path.
    const bind_str = "/tmp/dir\npath:/dst";
    const binds = [_][]const u8{bind_str};
    const config = docker.ContainerConfig{
        .image = "img",
        .name = "c",
        .env = &.{},
        .binds = &binds,
    };
    const body = try buildBody(alloc, config);
    defer alloc.free(body);

    // The raw newline byte must not appear in the body; it must be \n.
    try std.testing.expect(std.mem.indexOf(u8, body, "\\n") != null);
    // Confirm no raw newline inside the Binds array value.
    const binds_start = std.mem.indexOf(u8, body, "\"Binds\":[") orelse
        return error.MissingBinds;
    const binds_end = std.mem.indexOfPos(u8, body, binds_start, "]") orelse
        return error.MissingBindsEnd;
    const binds_section = body[binds_start..binds_end];
    try std.testing.expect(std.mem.indexOf(u8, binds_section, "\n") == null);
}

test "AC4: tab byte in bind path is escaped to \\t" {
    const alloc = std.testing.allocator;
    const bind_str = "/tmp/dir\tpath:/dst";
    const binds = [_][]const u8{bind_str};
    const config = docker.ContainerConfig{
        .image = "img",
        .name = "c",
        .env = &.{},
        .binds = &binds,
    };
    const body = try buildBody(alloc, config);
    defer alloc.free(body);

    try std.testing.expect(std.mem.indexOf(u8, body, "\\t") != null);
    // Confirm valid JSON round-trip.
    var parsed = try std.json.parseFromSlice(std.json.Value, alloc, body, .{});
    defer parsed.deinit();
    const host_config = parsed.value.object.get("HostConfig") orelse
        return error.MissingHostConfig;
    const binds_arr = host_config.object.get("Binds") orelse
        return error.MissingBinds;
    try std.testing.expectEqualStrings(bind_str, binds_arr.array.items[0].string);
}

test "AC4: control character 0x01 in bind path is escaped to \\u0001" {
    const alloc = std.testing.allocator;
    const bind_str = "/tmp/" ++ &[_]u8{0x01} ++ "path:/dst";
    const binds = [_][]const u8{bind_str};
    const config = docker.ContainerConfig{
        .image = "img",
        .name = "c",
        .env = &.{},
        .binds = &binds,
    };
    const body = try buildBody(alloc, config);
    defer alloc.free(body);

    // 0x01 must be emitted as \u0001
    try std.testing.expect(std.mem.indexOf(u8, body, "\\u0001") != null);
    // The raw byte must not appear.
    try std.testing.expect(std.mem.indexOf(u8, body, &[_]u8{0x01}) == null);
}

// =============================================================================
// AC5: Multiple binds are comma-separated and each is independently escaped
// =============================================================================

test "AC5: two plain binds produce a comma-separated JSON array" {
    const alloc = std.testing.allocator;
    const binds = [_][]const u8{
        "/home/user/project:/workspace",
        "/tmp/cache:/cache",
    };
    const config = docker.ContainerConfig{
        .image = "img",
        .name = "c",
        .env = &.{},
        .binds = &binds,
    };
    const body = try buildBody(alloc, config);
    defer alloc.free(body);

    var parsed = try std.json.parseFromSlice(std.json.Value, alloc, body, .{});
    defer parsed.deinit();

    const host_config = parsed.value.object.get("HostConfig") orelse
        return error.MissingHostConfig;
    const binds_arr = host_config.object.get("Binds") orelse
        return error.MissingBinds;
    try std.testing.expectEqual(@as(usize, 2), binds_arr.array.items.len);
    try std.testing.expectEqualStrings("/home/user/project:/workspace", binds_arr.array.items[0].string);
    try std.testing.expectEqualStrings("/tmp/cache:/cache", binds_arr.array.items[1].string);
}

test "AC5: three binds where second has special chars — all correctly escaped" {
    const alloc = std.testing.allocator;
    const binds = [_][]const u8{
        "/safe/path1:/dst1",
        "/path/with\"quote:/dst2",
        "/safe/path3:/dst3",
    };
    const config = docker.ContainerConfig{
        .image = "img",
        .name = "c",
        .env = &.{},
        .binds = &binds,
    };
    const body = try buildBody(alloc, config);
    defer alloc.free(body);

    var parsed = try std.json.parseFromSlice(std.json.Value, alloc, body, .{});
    defer parsed.deinit();

    const host_config = parsed.value.object.get("HostConfig") orelse
        return error.MissingHostConfig;
    const binds_arr = host_config.object.get("Binds") orelse
        return error.MissingBinds;
    try std.testing.expectEqual(@as(usize, 3), binds_arr.array.items.len);
    try std.testing.expectEqualStrings("/safe/path1:/dst1", binds_arr.array.items[0].string);
    try std.testing.expectEqualStrings("/path/with\"quote:/dst2", binds_arr.array.items[1].string);
    try std.testing.expectEqualStrings("/safe/path3:/dst3", binds_arr.array.items[2].string);
}

test "AC5: comma appears between elements but not before first or after last" {
    const alloc = std.testing.allocator;
    const binds = [_][]const u8{
        "/a:/b",
        "/c:/d",
    };
    const config = docker.ContainerConfig{
        .image = "img",
        .name = "c",
        .env = &.{},
        .binds = &binds,
    };
    const body = try buildBody(alloc, config);
    defer alloc.free(body);

    // Expect exactly: "Binds":["/a:/b","/c:/d"]
    try std.testing.expect(std.mem.indexOf(u8, body, "\"Binds\":[\"/a:/b\",\"/c:/d\"]") != null);
}

// =============================================================================
// AC6: Empty binds slice → "Binds":[]
// =============================================================================

test "AC6: empty binds slice produces Binds:[] in body" {
    const alloc = std.testing.allocator;
    const config = docker.ContainerConfig{
        .image = "img",
        .name = "c",
        .env = &.{},
        .binds = &.{},
    };
    const body = try buildBody(alloc, config);
    defer alloc.free(body);

    try std.testing.expect(std.mem.indexOf(u8, body, "\"Binds\":[]") != null);
}

test "AC6: empty binds produce valid JSON" {
    const alloc = std.testing.allocator;
    const config = docker.ContainerConfig{
        .image = "img",
        .name = "c",
        .env = &.{},
        .binds = &.{},
    };
    const body = try buildBody(alloc, config);
    defer alloc.free(body);

    var parsed = try std.json.parseFromSlice(std.json.Value, alloc, body, .{});
    defer parsed.deinit();

    const host_config = parsed.value.object.get("HostConfig") orelse
        return error.MissingHostConfig;
    const binds_arr = host_config.object.get("Binds") orelse
        return error.MissingBinds;
    try std.testing.expectEqual(@as(usize, 0), binds_arr.array.items.len);
}

// =============================================================================
// AC7: Env-var escaping is unchanged
// =============================================================================

test "AC7: env var with double-quote is still correctly escaped" {
    const alloc = std.testing.allocator;
    const env_vars = [_][]const u8{"MY_VAR=value\"with\"quotes"};
    const config = docker.ContainerConfig{
        .image = "img",
        .name = "c",
        .env = &env_vars,
        .binds = &.{},
    };
    const body = try buildBody(alloc, config);
    defer alloc.free(body);

    var parsed = try std.json.parseFromSlice(std.json.Value, alloc, body, .{});
    defer parsed.deinit();

    const env_arr = parsed.value.object.get("Env") orelse
        return error.MissingEnv;
    try std.testing.expectEqual(@as(usize, 1), env_arr.array.items.len);
    try std.testing.expectEqualStrings("MY_VAR=value\"with\"quotes", env_arr.array.items[0].string);
}

test "AC7: env var with backslash is still correctly escaped" {
    const alloc = std.testing.allocator;
    const env_vars = [_][]const u8{"PATH=C:\\Windows\\System32"};
    const config = docker.ContainerConfig{
        .image = "img",
        .name = "c",
        .env = &env_vars,
        .binds = &.{},
    };
    const body = try buildBody(alloc, config);
    defer alloc.free(body);

    var parsed = try std.json.parseFromSlice(std.json.Value, alloc, body, .{});
    defer parsed.deinit();

    const env_arr = parsed.value.object.get("Env") orelse
        return error.MissingEnv;
    try std.testing.expectEqualStrings("PATH=C:\\Windows\\System32", env_arr.array.items[0].string);
}

test "AC7: both env and bind special chars are independently escaped" {
    const alloc = std.testing.allocator;
    const env_vars = [_][]const u8{"K=v\"al"};
    const binds = [_][]const u8{"/path/with\\slash:/dst"};
    const config = docker.ContainerConfig{
        .image = "img",
        .name = "c",
        .env = &env_vars,
        .binds = &binds,
    };
    const body = try buildBody(alloc, config);
    defer alloc.free(body);

    var parsed = try std.json.parseFromSlice(std.json.Value, alloc, body, .{});
    defer parsed.deinit();

    const env_arr = parsed.value.object.get("Env") orelse
        return error.MissingEnv;
    try std.testing.expectEqualStrings("K=v\"al", env_arr.array.items[0].string);

    const host_config = parsed.value.object.get("HostConfig") orelse
        return error.MissingHostConfig;
    const binds_arr = host_config.object.get("Binds") orelse
        return error.MissingBinds;
    try std.testing.expectEqualStrings("/path/with\\slash:/dst", binds_arr.array.items[0].string);
}

// =============================================================================
// E1: Literal two-char backslash-n (not a newline byte)
//     `\` → `\\`; `n` → `n` → together `\\n` in the JSON body.
// =============================================================================

test "E1: literal backslash-n two-char sequence is escaped as \\\\n" {
    const alloc = std.testing.allocator;
    // host path contains the two bytes: 0x5C 0x6E  (\  n)
    const bind_str = "/tmp/\\npath:/dst";
    const binds = [_][]const u8{bind_str};
    const config = docker.ContainerConfig{
        .image = "img",
        .name = "c",
        .env = &.{},
        .binds = &binds,
    };
    const body = try buildBody(alloc, config);
    defer alloc.free(body);

    // In the JSON body the backslash becomes \\ and the n stays n → \\n
    // i.e., the raw bytes in body are: \ \ n
    try std.testing.expect(std.mem.indexOf(u8, body, "\\\\n") != null);

    // Round-trip: the parser must give back the original two bytes.
    var parsed = try std.json.parseFromSlice(std.json.Value, alloc, body, .{});
    defer parsed.deinit();

    const host_config = parsed.value.object.get("HostConfig") orelse
        return error.MissingHostConfig;
    const binds_arr = host_config.object.get("Binds") orelse
        return error.MissingBinds;
    try std.testing.expectEqualStrings(bind_str, binds_arr.array.items[0].string);
}

// =============================================================================
// E2: Single bind with special char — comma-skip logic (i == 0) still works
// =============================================================================

test "E2: single bind with double-quote has no leading or trailing comma" {
    const alloc = std.testing.allocator;
    const binds = [_][]const u8{"/dir\"name:/dst"};
    const config = docker.ContainerConfig{
        .image = "img",
        .name = "c",
        .env = &.{},
        .binds = &binds,
    };
    const body = try buildBody(alloc, config);
    defer alloc.free(body);

    // The Binds array must not start with a comma: "[," is forbidden.
    try std.testing.expect(std.mem.indexOf(u8, body, "[,") == null);
    // The Binds array must not end with a trailing comma before "]".
    try std.testing.expect(std.mem.indexOf(u8, body, ",]") == null);

    // Valid JSON parse.
    var parsed = try std.json.parseFromSlice(std.json.Value, alloc, body, .{});
    defer parsed.deinit();

    const host_config = parsed.value.object.get("HostConfig") orelse
        return error.MissingHostConfig;
    const binds_arr = host_config.object.get("Binds") orelse
        return error.MissingBinds;
    try std.testing.expectEqual(@as(usize, 1), binds_arr.array.items.len);
    try std.testing.expectEqualStrings("/dir\"name:/dst", binds_arr.array.items[0].string);
}

// =============================================================================
// E3: Unicode / multi-byte UTF-8 passthrough — no double-escaping
// =============================================================================

test "E3: unicode bind path passes through without double-escaping" {
    const alloc = std.testing.allocator;
    // UTF-8 encoded path with multi-byte characters.
    const bind_str = "/tmp/héllo_世界:/dst";
    const binds = [_][]const u8{bind_str};
    const config = docker.ContainerConfig{
        .image = "img",
        .name = "c",
        .env = &.{},
        .binds = &binds,
    };
    const body = try buildBody(alloc, config);
    defer alloc.free(body);

    // The UTF-8 bytes must appear verbatim in the body (no \uXXXX escaping).
    try std.testing.expect(std.mem.indexOf(u8, body, "héllo") != null);
    try std.testing.expect(std.mem.indexOf(u8, body, "世界") != null);

    // Valid JSON parse and round-trip.
    var parsed = try std.json.parseFromSlice(std.json.Value, alloc, body, .{});
    defer parsed.deinit();

    const host_config = parsed.value.object.get("HostConfig") orelse
        return error.MissingHostConfig;
    const binds_arr = host_config.object.get("Binds") orelse
        return error.MissingBinds;
    try std.testing.expectEqualStrings(bind_str, binds_arr.array.items[0].string);
}

// =============================================================================
// Structural checks: required JSON fields are always present
// =============================================================================

test "Struct: body always contains Image, Tty, Env, HostConfig, Binds keys" {
    const alloc = std.testing.allocator;
    const config = docker.ContainerConfig{
        .image = "myimage:1.0",
        .name = "mycontainer",
        .env = &.{"FOO=bar"},
        .binds = &.{"/a:/b"},
    };
    const body = try buildBody(alloc, config);
    defer alloc.free(body);

    var parsed = try std.json.parseFromSlice(std.json.Value, alloc, body, .{});
    defer parsed.deinit();

    try std.testing.expect(parsed.value.object.get("Image") != null);
    try std.testing.expect(parsed.value.object.get("Env") != null);
    const hc = parsed.value.object.get("HostConfig") orelse return error.MissingHostConfig;
    try std.testing.expect(hc.object.get("Binds") != null);
    try std.testing.expect(hc.object.get("Memory") != null);
    try std.testing.expect(hc.object.get("PidsLimit") != null);
}

test "Struct: image name appears correctly in body" {
    const alloc = std.testing.allocator;
    const config = docker.ContainerConfig{
        .image = "borg-agent:latest",
        .name = "borg-0",
        .env = &.{},
        .binds = &.{},
    };
    const body = try buildBody(alloc, config);
    defer alloc.free(body);

    var parsed = try std.json.parseFromSlice(std.json.Value, alloc, body, .{});
    defer parsed.deinit();

    const image = parsed.value.object.get("Image") orelse return error.MissingImage;
    try std.testing.expectEqualStrings("borg-agent:latest", image.string);
}
