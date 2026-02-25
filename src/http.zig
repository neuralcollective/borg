const std = @import("std");

pub const Header = struct {
    name: []const u8,
    value: []const u8,
};

pub const Response = struct {
    status: std.http.Status,
    body: []const u8,
    allocator: std.mem.Allocator,

    pub fn deinit(self: *Response) void {
        self.allocator.free(self.body);
    }
};

pub fn get(allocator: std.mem.Allocator, url_str: []const u8) !Response {
    return request(allocator, .GET, url_str, null, &.{});
}

pub fn post(allocator: std.mem.Allocator, url_str: []const u8, body: ?[]const u8, extra_headers: []const Header) !Response {
    return request(allocator, .POST, url_str, body, extra_headers);
}

pub fn postJson(allocator: std.mem.Allocator, url_str: []const u8, body: []const u8) !Response {
    return request(allocator, .POST, url_str, body, &.{
        .{ .name = "Content-Type", .value = "application/json" },
    });
}

fn request(allocator: std.mem.Allocator, method: std.http.Method, url_str: []const u8, body: ?[]const u8, extra_headers: []const Header) !Response {
    var client = std.http.Client{ .allocator = allocator };
    defer client.deinit();

    const uri = try std.Uri.parse(url_str);

    var header_buf: [16384]u8 = undefined;
    var req = try client.open(method, uri, .{
        .server_header_buffer = &header_buf,
        .extra_headers = @ptrCast(extra_headers),
    });
    defer req.deinit();

    if (body) |b| {
        req.transfer_encoding = .{ .content_length = b.len };
    }

    try req.send();

    if (body) |b| {
        try req.writer().writeAll(b);
        try req.finish();
    }

    try req.wait();

    const resp_body = try req.reader().readAllAlloc(allocator, 10 * 1024 * 1024);

    return Response{
        .status = req.response.status,
        .body = resp_body,
        .allocator = allocator,
    };
}

/// HTTP client for Unix domain sockets (Docker API)
pub fn unixRequest(allocator: std.mem.Allocator, socket_path: []const u8, method: std.http.Method, path: []const u8, body: ?[]const u8) !Response {
    const stream = try std.net.connectUnixSocket(socket_path);
    defer stream.close();

    // Build raw HTTP request
    var req_buf = std.ArrayList(u8).init(allocator);
    defer req_buf.deinit();

    const method_str = switch (method) {
        .GET => "GET",
        .POST => "POST",
        .DELETE => "DELETE",
        .PUT => "PUT",
        else => "GET",
    };

    try req_buf.writer().print("{s} {s} HTTP/1.1\r\nHost: localhost\r\n", .{ method_str, path });

    if (body) |b| {
        try req_buf.writer().print("Content-Type: application/json\r\nContent-Length: {d}\r\n", .{b.len});
    }

    try req_buf.writer().writeAll("Connection: close\r\n\r\n");

    if (body) |b| {
        try req_buf.writer().writeAll(b);
    }

    try stream.writeAll(req_buf.items);

    // Read response
    var resp_buf = std.ArrayList(u8).init(allocator);
    defer resp_buf.deinit();

    var read_buf: [8192]u8 = undefined;
    while (true) {
        const n = stream.read(&read_buf) catch |err| switch (err) {
            error.ConnectionResetByPeer => break,
            else => return err,
        };
        if (n == 0) break;
        try resp_buf.appendSlice(read_buf[0..n]);
    }

    // Parse status line and find body
    const resp_data = resp_buf.items;
    var status: std.http.Status = .ok;

    // Find end of headers
    if (std.mem.indexOf(u8, resp_data, "\r\n\r\n")) |header_end| {
        // Parse status code from first line
        const first_line_end = std.mem.indexOf(u8, resp_data, "\r\n") orelse header_end;
        const first_line = resp_data[0..first_line_end];
        // "HTTP/1.1 200 OK"
        if (first_line.len > 9) {
            const code_str = first_line[9..12];
            const code = std.fmt.parseInt(u10, code_str, 10) catch 200;
            status = @enumFromInt(code);
        }

        const body_start = header_end + 4;

        // Handle chunked transfer encoding
        if (std.mem.indexOf(u8, resp_data[0..header_end], "Transfer-Encoding: chunked") != null) {
            const decoded = try decodeChunked(allocator, resp_data[body_start..]);
            return Response{
                .status = status,
                .body = decoded,
                .allocator = allocator,
            };
        }

        const resp_body = try allocator.dupe(u8, resp_data[body_start..]);
        return Response{
            .status = status,
            .body = resp_body,
            .allocator = allocator,
        };
    }

    return Response{
        .status = status,
        .body = try allocator.dupe(u8, ""),
        .allocator = allocator,
    };
}

fn decodeChunked(allocator: std.mem.Allocator, data: []const u8) ![]u8 {
    var result = std.ArrayList(u8).init(allocator);
    var pos: usize = 0;

    while (pos < data.len) {
        // Find end of chunk size line
        const line_end = std.mem.indexOf(u8, data[pos..], "\r\n") orelse break;
        const size_str = std.mem.trim(u8, data[pos .. pos + line_end], &[_]u8{ ' ', '\t' });
        const chunk_size = std.fmt.parseInt(usize, size_str, 16) catch break;

        if (chunk_size == 0) break;

        pos += line_end + 2;
        if (pos + chunk_size > data.len) break;

        try result.appendSlice(data[pos .. pos + chunk_size]);
        pos += chunk_size + 2; // skip \r\n after chunk data
    }

    return result.toOwnedSlice();
}

// ── Tests ──────────────────────────────────────────────────────────────

test "decodeChunked reassembles chunks" {
    const alloc = std.testing.allocator;
    const chunked = "4\r\nWiki\r\n5\r\npedia\r\n0\r\n\r\n";
    const result = try decodeChunked(alloc, chunked);
    defer alloc.free(result);
    try std.testing.expectEqualStrings("Wikipedia", result);
}

test "decodeChunked handles single chunk" {
    const alloc = std.testing.allocator;
    const chunked = "d\r\nHello, World!\r\n0\r\n\r\n";
    const result = try decodeChunked(alloc, chunked);
    defer alloc.free(result);
    try std.testing.expectEqualStrings("Hello, World!", result);
}

test "decodeChunked empty input" {
    const alloc = std.testing.allocator;
    const result = try decodeChunked(alloc, "");
    defer alloc.free(result);
    try std.testing.expectEqualStrings("", result);
}

test "decodeChunked immediate zero-size chunk" {
    const alloc = std.testing.allocator;
    const result = try decodeChunked(alloc, "0\r\n\r\n");
    defer alloc.free(result);
    try std.testing.expectEqualStrings("", result);
}

test "decodeChunked malformed hex chunk size" {
    const alloc = std.testing.allocator;
    const result = try decodeChunked(alloc, "xyz\r\ndata\r\n0\r\n\r\n");
    defer alloc.free(result);
    try std.testing.expectEqualStrings("", result);
}

test "decodeChunked truncated chunk data" {
    const alloc = std.testing.allocator;
    // chunk size is 0xa=10 but only 5 bytes of payload follow
    const result = try decodeChunked(alloc, "a\r\nhello\r\n");
    defer alloc.free(result);
    try std.testing.expectEqualStrings("", result);
}

test "decodeChunked missing separator between chunks" {
    const alloc = std.testing.allocator;
    // "Wiki" (4 bytes) is decoded; the absent \r\n causes pos to land on "pedia"
    // which is invalid hex, so catch break fires and only "Wiki" is returned
    const result = try decodeChunked(alloc, "4\r\nWiki5\r\npedia\r\n0\r\n\r\n");
    defer alloc.free(result);
    try std.testing.expectEqualStrings("Wiki", result);
}

// ── Uppercase / mixed-case hex chunk sizes ─────────────────────────────

test "decodeChunked uppercase hex chunk size" {
    // 'A' == 10 decimal; payload is exactly 10 bytes
    const alloc = std.testing.allocator;
    const chunked = "A\r\n0123456789\r\n0\r\n\r\n";
    const result = try decodeChunked(alloc, chunked);
    defer alloc.free(result);
    try std.testing.expectEqualStrings("0123456789", result);
}

test "decodeChunked uppercase hex chunk size F" {
    // 'F' == 15 decimal; payload is exactly 15 bytes
    const alloc = std.testing.allocator;
    const chunked = "F\r\nABCDEFGHIJKLMNO\r\n0\r\n\r\n";
    const result = try decodeChunked(alloc, chunked);
    defer alloc.free(result);
    try std.testing.expectEqualStrings("ABCDEFGHIJKLMNO", result);
}

test "decodeChunked uppercase multi-chunk" {
    // First chunk: 4 bytes ("Wiki"), second chunk: FF (255 bytes)
    const alloc = std.testing.allocator;
    var chunked = std.ArrayList(u8).init(alloc);
    defer chunked.deinit();

    // Build a 255-byte payload of 'x'
    const payload = "x" ** 255;

    try chunked.appendSlice("4\r\nWiki\r\n");
    try chunked.appendSlice("FF\r\n");
    try chunked.appendSlice(payload);
    try chunked.appendSlice("\r\n0\r\n\r\n");

    const result = try decodeChunked(alloc, chunked.items);
    defer alloc.free(result);

    try std.testing.expectEqual(@as(usize, 4 + 255), result.len);
    try std.testing.expectEqualStrings("Wiki", result[0..4]);
    try std.testing.expectEqualStrings(payload, result[4..]);
}

test "decodeChunked mixed-case hex chunk size" {
    // 'aB' == 0xAB == 171 decimal
    const alloc = std.testing.allocator;
    const payload = "y" ** 171;
    var chunked = std.ArrayList(u8).init(alloc);
    defer chunked.deinit();
    try chunked.appendSlice("aB\r\n");
    try chunked.appendSlice(payload);
    try chunked.appendSlice("\r\n0\r\n\r\n");

    const result = try decodeChunked(alloc, chunked.items);
    defer alloc.free(result);

    try std.testing.expectEqual(@as(usize, 171), result.len);
    try std.testing.expectEqualStrings(payload, result);
}

test "decodeChunked uppercase terminating zero after uppercase chunk" {
    // Ensure terminating 0\r\n\r\n produces empty result after an uppercase-sized chunk
    const alloc = std.testing.allocator;
    // 'B' == 11 bytes
    const chunked = "B\r\nhello world\r\n0\r\n\r\n";
    const result = try decodeChunked(alloc, chunked);
    defer alloc.free(result);
    try std.testing.expectEqualStrings("hello world", result);
    // Confirming termination: nothing after the 11-byte chunk
    try std.testing.expectEqual(@as(usize, 11), result.len);
}

test "decodeChunked mixed-case terminating zero" {
    // 'Ab' == 0xAB == 171; terminating 0\r\n\r\n must produce no extra bytes
    const alloc = std.testing.allocator;
    const payload = "z" ** 171;
    var chunked = std.ArrayList(u8).init(alloc);
    defer chunked.deinit();
    try chunked.appendSlice("Ab\r\n");
    try chunked.appendSlice(payload);
    try chunked.appendSlice("\r\n0\r\n\r\n");

    const result = try decodeChunked(alloc, chunked.items);
    defer alloc.free(result);

    // Only the 171-byte payload; the terminating chunk adds nothing
    try std.testing.expectEqual(@as(usize, 171), result.len);
    try std.testing.expectEqualStrings(payload, result);
}

test "decodeChunked uppercase FF equals lowercase ff equals mixed-case Ff" {
    // All three representations of 255 must yield identical decoded output
    const alloc = std.testing.allocator;
    const payload = "q" ** 255;

    const build_input = struct {
        fn f(a: std.mem.Allocator, size_str: []const u8, p: []const u8) ![]u8 {
            var buf = std.ArrayList(u8).init(a);
            try buf.appendSlice(size_str);
            try buf.appendSlice("\r\n");
            try buf.appendSlice(p);
            try buf.appendSlice("\r\n0\r\n\r\n");
            return buf.toOwnedSlice();
        }
    };

    const input_upper = try build_input.f(alloc, "FF", payload);
    defer alloc.free(input_upper);
    const input_lower = try build_input.f(alloc, "ff", payload);
    defer alloc.free(input_lower);
    const input_mixed = try build_input.f(alloc, "Ff", payload);
    defer alloc.free(input_mixed);

    const res_upper = try decodeChunked(alloc, input_upper);
    defer alloc.free(res_upper);
    const res_lower = try decodeChunked(alloc, input_lower);
    defer alloc.free(res_lower);
    const res_mixed = try decodeChunked(alloc, input_mixed);
    defer alloc.free(res_mixed);

    try std.testing.expectEqualStrings(payload, res_upper);
    try std.testing.expectEqualStrings(payload, res_lower);
    try std.testing.expectEqualStrings(payload, res_mixed);
    try std.testing.expectEqualStrings(res_upper, res_lower);
    try std.testing.expectEqualStrings(res_upper, res_mixed);
}
