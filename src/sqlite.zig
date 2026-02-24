const std = @import("std");
const c = @cImport({
    @cInclude("sqlite3.h");
});

pub const SqliteError = error{
    OpenFailed,
    PrepareFailed,
    StepFailed,
    BindFailed,
    ExecFailed,
    OutOfMemory,
};

pub const Row = struct {
    columns: [][]const u8,
    allocator: std.mem.Allocator,

    pub fn get(self: Row, idx: usize) ?[]const u8 {
        if (idx >= self.columns.len) return null;
        return self.columns[idx];
    }

    pub fn getInt(self: Row, idx: usize) ?i64 {
        const text = self.get(idx) orelse return null;
        return std.fmt.parseInt(i64, text, 10) catch null;
    }
};

pub const Rows = struct {
    items: []Row,
    allocator: std.mem.Allocator,

    pub fn deinit(self: *Rows) void {
        for (self.items) |row| {
            for (row.columns) |col| {
                row.allocator.free(col);
            }
            row.allocator.free(row.columns);
        }
        self.allocator.free(self.items);
    }
};

pub const Database = struct {
    db: *c.sqlite3,

    pub fn open(path: [:0]const u8) SqliteError!Database {
        var db: ?*c.sqlite3 = null;
        const rc = c.sqlite3_open(path.ptr, &db);
        if (rc != c.SQLITE_OK or db == null) {
            if (db) |d| _ = c.sqlite3_close(d);
            return SqliteError.OpenFailed;
        }
        // WAL mode for concurrent reads
        _ = c.sqlite3_exec(db.?, "PRAGMA journal_mode=WAL;", null, null, null);
        _ = c.sqlite3_exec(db.?, "PRAGMA foreign_keys=ON;", null, null, null);
        return Database{ .db = db.? };
    }

    pub fn close(self: *Database) void {
        _ = c.sqlite3_close(self.db);
    }

    pub fn exec(self: *Database, sql: [*:0]const u8) SqliteError!void {
        var err_msg: [*c]u8 = null;
        const rc = c.sqlite3_exec(self.db, sql, null, null, &err_msg);
        if (err_msg) |msg| {
            std.log.err("SQLite exec error: {s}", .{msg});
            c.sqlite3_free(msg);
        }
        if (rc != c.SQLITE_OK) return SqliteError.ExecFailed;
    }

    pub fn query(self: *Database, allocator: std.mem.Allocator, sql: [*:0]const u8, params: anytype) SqliteError!Rows {
        var stmt: ?*c.sqlite3_stmt = null;
        var rc = c.sqlite3_prepare_v2(self.db, sql, -1, &stmt, null);
        if (rc != c.SQLITE_OK or stmt == null) {
            std.log.err("SQLite prepare error: {s}", .{c.sqlite3_errmsg(self.db)});
            return SqliteError.PrepareFailed;
        }
        defer _ = c.sqlite3_finalize(stmt);

        // Bind parameters
        inline for (params, 0..) |param, i| {
            const idx: c_int = @intCast(i + 1);
            const T = @TypeOf(param);
            if (T == []const u8 or T == [:0]const u8) {
                rc = c.sqlite3_bind_text(stmt.?, idx, param.ptr, @intCast(param.len), c.SQLITE_TRANSIENT);
            } else if (@typeInfo(T) == .int or @typeInfo(T) == .comptime_int) {
                rc = c.sqlite3_bind_int64(stmt.?, idx, @intCast(param));
            } else if (@typeInfo(T) == .optional) {
                if (param) |val| {
                    const Inner = @TypeOf(val);
                    if (Inner == []const u8 or Inner == [:0]const u8) {
                        rc = c.sqlite3_bind_text(stmt.?, idx, val.ptr, @intCast(val.len), c.SQLITE_TRANSIENT);
                    } else {
                        rc = c.sqlite3_bind_int64(stmt.?, idx, @intCast(val));
                    }
                } else {
                    rc = c.sqlite3_bind_null(stmt.?, idx);
                }
            }
            if (rc != c.SQLITE_OK) return SqliteError.BindFailed;
        }

        var rows = std.ArrayList(Row).init(allocator);
        const col_count: usize = @intCast(c.sqlite3_column_count(stmt.?));

        while (c.sqlite3_step(stmt.?) == c.SQLITE_ROW) {
            var columns = try allocator.alloc([]const u8, col_count);
            for (0..col_count) |col_idx| {
                const ci: c_int = @intCast(col_idx);
                const text_ptr = c.sqlite3_column_text(stmt.?, ci);
                if (text_ptr) |tp| {
                    const len: usize = @intCast(c.sqlite3_column_bytes(stmt.?, ci));
                    const duped = try allocator.alloc(u8, len);
                    @memcpy(duped, tp[0..len]);
                    columns[col_idx] = duped;
                } else {
                    columns[col_idx] = try allocator.dupe(u8, "");
                }
            }
            try rows.append(Row{ .columns = columns, .allocator = allocator });
        }

        return Rows{
            .items = try rows.toOwnedSlice(),
            .allocator = allocator,
        };
    }

    pub fn execute(self: *Database, sql: [*:0]const u8, params: anytype) SqliteError!void {
        var stmt: ?*c.sqlite3_stmt = null;
        var rc = c.sqlite3_prepare_v2(self.db, sql, -1, &stmt, null);
        if (rc != c.SQLITE_OK or stmt == null) {
            std.log.err("SQLite prepare error: {s}", .{c.sqlite3_errmsg(self.db)});
            return SqliteError.PrepareFailed;
        }
        defer _ = c.sqlite3_finalize(stmt);

        inline for (params, 0..) |param, i| {
            const idx: c_int = @intCast(i + 1);
            const T = @TypeOf(param);
            if (T == []const u8 or T == [:0]const u8) {
                rc = c.sqlite3_bind_text(stmt.?, idx, param.ptr, @intCast(param.len), c.SQLITE_TRANSIENT);
            } else if (@typeInfo(T) == .int or @typeInfo(T) == .comptime_int) {
                rc = c.sqlite3_bind_int64(stmt.?, idx, @intCast(param));
            } else if (@typeInfo(T) == .optional) {
                if (param) |val| {
                    const Inner = @TypeOf(val);
                    if (Inner == []const u8 or Inner == [:0]const u8) {
                        rc = c.sqlite3_bind_text(stmt.?, idx, val.ptr, @intCast(val.len), c.SQLITE_TRANSIENT);
                    } else {
                        rc = c.sqlite3_bind_int64(stmt.?, idx, @intCast(val));
                    }
                } else {
                    rc = c.sqlite3_bind_null(stmt.?, idx);
                }
            }
            if (rc != c.SQLITE_OK) return SqliteError.BindFailed;
        }

        rc = c.sqlite3_step(stmt.?);
        if (rc != c.SQLITE_DONE and rc != c.SQLITE_ROW) {
            std.log.err("SQLite step error: {s}", .{c.sqlite3_errmsg(self.db)});
            return SqliteError.StepFailed;
        }
    }

    pub fn lastInsertRowId(self: *Database) i64 {
        return c.sqlite3_last_insert_rowid(self.db);
    }
};
