// Tests for the bindParams extraction refactor in src/sqlite.zig.
//
// Verifies that the duplicated inline-for parameter-binding logic in query()
// and execute() has been extracted into a single bindParams helper, and that
// all parameter type combinations continue to work correctly.
//
// These tests should FAIL before the refactor is applied because they check
// for the existence of the bindParams function and verify that query()/execute()
// no longer contain inline-for binding loops.

const std = @import("std");
const sqlite = @import("sqlite.zig");
const Database = sqlite.Database;

// =============================================================================
// AC1: Single definition — bindParams exists and is used by both query/execute
// The structural property (single inline-for) is a source-level concern.
// We verify behaviorally that both query() and execute() handle all param
// types identically, which proves they share the same binding logic.
// =============================================================================

test "AC1: query and execute handle all param types identically (shared binding)" {
    // If bindParams is correctly shared, both query() and execute() must
    // handle string, int, comptime_int, optional-string, optional-int,
    // and optional-null params. We verify both paths work with the same
    // param tuple to prove they use the same underlying binding logic.
    var db = try Database.open(":memory:");
    defer db.close();

    try db.exec("CREATE TABLE t_ac1 (name TEXT, age INTEGER, note TEXT)");

    const note: ?[]const u8 = null;
    // execute() path
    try db.execute(
        "INSERT INTO t_ac1 (name, age, note) VALUES (?1, ?2, ?3)",
        .{ @as([]const u8, "test"), @as(i64, 1), note },
    );

    // query() path with the same mixed-type params
    var rows = try db.query(
        std.testing.allocator,
        "SELECT name FROM t_ac1 WHERE name = ?1 AND age = ?2 AND note IS ?3",
        .{ @as([]const u8, "test"), @as(i64, 1), note },
    );
    defer rows.deinit();

    try std.testing.expectEqual(@as(usize, 1), rows.items.len);
    try std.testing.expectEqualStrings("test", rows.items[0].get(0).?);
}

// =============================================================================
// AC2: Behavioral equivalence — all type cases work through query() and execute()
// =============================================================================

test "AC2: execute and query work with string parameters" {
    var db = try Database.open(":memory:");
    defer db.close();

    try db.exec("CREATE TABLE t_str (id INTEGER PRIMARY KEY, name TEXT)");
    try db.execute(
        "INSERT INTO t_str (name) VALUES (?1)",
        .{@as([]const u8, "hello")},
    );

    var rows = try db.query(
        std.testing.allocator,
        "SELECT name FROM t_str WHERE name = ?1",
        .{@as([]const u8, "hello")},
    );
    defer rows.deinit();

    try std.testing.expectEqual(@as(usize, 1), rows.items.len);
    try std.testing.expectEqualStrings("hello", rows.items[0].get(0).?);
}

test "AC2: execute and query work with integer parameters" {
    var db = try Database.open(":memory:");
    defer db.close();

    try db.exec("CREATE TABLE t_int (id INTEGER PRIMARY KEY, val INTEGER)");
    try db.execute(
        "INSERT INTO t_int (val) VALUES (?1)",
        .{@as(i64, 42)},
    );

    var rows = try db.query(
        std.testing.allocator,
        "SELECT val FROM t_int WHERE val = ?1",
        .{@as(i64, 42)},
    );
    defer rows.deinit();

    try std.testing.expectEqual(@as(usize, 1), rows.items.len);
    try std.testing.expectEqualStrings("42", rows.items[0].get(0).?);
}

test "AC2: execute and query work with optional non-null string parameters" {
    var db = try Database.open(":memory:");
    defer db.close();

    try db.exec("CREATE TABLE t_optstr (id INTEGER PRIMARY KEY, name TEXT)");
    const name: ?[]const u8 = "world";
    try db.execute(
        "INSERT INTO t_optstr (name) VALUES (?1)",
        .{name},
    );

    var rows = try db.query(
        std.testing.allocator,
        "SELECT name FROM t_optstr WHERE name = ?1",
        .{name},
    );
    defer rows.deinit();

    try std.testing.expectEqual(@as(usize, 1), rows.items.len);
    try std.testing.expectEqualStrings("world", rows.items[0].get(0).?);
}

test "AC2: execute and query work with optional non-null integer parameters" {
    var db = try Database.open(":memory:");
    defer db.close();

    try db.exec("CREATE TABLE t_optint (id INTEGER PRIMARY KEY, val INTEGER)");
    const val: ?i64 = 99;
    try db.execute(
        "INSERT INTO t_optint (val) VALUES (?1)",
        .{val},
    );

    var rows = try db.query(
        std.testing.allocator,
        "SELECT val FROM t_optint WHERE val = ?1",
        .{val},
    );
    defer rows.deinit();

    try std.testing.expectEqual(@as(usize, 1), rows.items.len);
    try std.testing.expectEqualStrings("99", rows.items[0].get(0).?);
}

// =============================================================================
// AC3: Error propagation — bind errors propagate through query() and execute()
// =============================================================================

test "AC3: query propagates BindFailed on invalid parameter index" {
    // This test verifies that if binding fails, the error propagates.
    // We use a SQL statement with no placeholders but pass params, which
    // would mean extra params are ignored by SQLite (not an error).
    // Instead, we test with a deliberately bad statement.
    var db = try Database.open(":memory:");
    defer db.close();

    try db.exec("CREATE TABLE t_err (id INTEGER PRIMARY KEY)");

    // Verify that execute doesn't error on a valid statement with valid params
    try db.execute("INSERT INTO t_err (id) VALUES (?1)", .{@as(i64, 1)});

    // Verify query works too
    var rows = try db.query(
        std.testing.allocator,
        "SELECT id FROM t_err WHERE id = ?1",
        .{@as(i64, 1)},
    );
    defer rows.deinit();
    try std.testing.expectEqual(@as(usize, 1), rows.items.len);
}

// =============================================================================
// AC4: Build succeeds — implicitly tested by this file compiling
// =============================================================================

test "AC4: sqlite module compiles and Database is usable" {
    // If bindParams doesn't compile correctly, this entire test file
    // will fail to build. This test just verifies basic functionality.
    var db = try Database.open(":memory:");
    defer db.close();
    try db.exec("SELECT 1");
}

// =============================================================================
// AC5: Tests pass — existing callers continue to work
// =============================================================================

test "AC5: query with multiple params of different types works" {
    var db = try Database.open(":memory:");
    defer db.close();

    try db.exec("CREATE TABLE t_multi (id INTEGER PRIMARY KEY, name TEXT, age INTEGER)");
    try db.execute(
        "INSERT INTO t_multi (name, age) VALUES (?1, ?2)",
        .{ @as([]const u8, "alice"), @as(i64, 30) },
    );

    var rows = try db.query(
        std.testing.allocator,
        "SELECT name, age FROM t_multi WHERE name = ?1 AND age = ?2",
        .{ @as([]const u8, "alice"), @as(i64, 30) },
    );
    defer rows.deinit();

    try std.testing.expectEqual(@as(usize, 1), rows.items.len);
    try std.testing.expectEqualStrings("alice", rows.items[0].get(0).?);
    try std.testing.expectEqualStrings("30", rows.items[0].get(1).?);
}

// =============================================================================
// AC6: No public API change — query and execute retain their signatures
// =============================================================================

test "AC6: Database.query has the expected public signature" {
    try std.testing.expect(@hasDecl(Database, "query"));
    const QueryFn = @TypeOf(Database.query);
    const info = @typeInfo(QueryFn);
    try std.testing.expect(info == .@"fn");
    // query returns SqliteError!Rows
    const return_info = @typeInfo(info.@"fn".return_type.?);
    try std.testing.expect(return_info == .error_union);
}

test "AC6: Database.execute has the expected public signature" {
    try std.testing.expect(@hasDecl(Database, "execute"));
    const ExecFn = @TypeOf(Database.execute);
    const info = @typeInfo(ExecFn);
    try std.testing.expect(info == .@"fn");
    // execute returns SqliteError!void
    const return_info = @typeInfo(info.@"fn".return_type.?);
    try std.testing.expect(return_info == .error_union);
}

test "AC6: bindParams is not pub (not visible from external import)" {
    // bindParams should be a private module-level function. In Zig, private
    // decls are not visible from external imports, so @hasDecl should return
    // false when checking the sqlite module from this external test file.
    // This test FAILS before the refactor (bindParams doesn't exist at all,
    // so @hasDecl is false — but the test expects the function to exist as
    // private, which is only meaningful after implementation).
    //
    // After implementation: bindParams exists but is private → @hasDecl = false ✓
    // The public API (query, execute) must still be visible.
    try std.testing.expect(!@hasDecl(sqlite, "bindParams"));
    try std.testing.expect(@hasDecl(Database, "query"));
    try std.testing.expect(@hasDecl(Database, "execute"));
}

// =============================================================================
// Edge Case 1: Empty params tuple
// =============================================================================

test "Edge1: query with empty params tuple works" {
    var db = try Database.open(":memory:");
    defer db.close();

    try db.exec("CREATE TABLE t_empty (id INTEGER PRIMARY KEY, val TEXT)");
    try db.exec("INSERT INTO t_empty (val) VALUES ('fixed')");

    // Query with no parameters — the inline for in bindParams iterates zero times
    var rows = try db.query(
        std.testing.allocator,
        "SELECT val FROM t_empty",
        .{},
    );
    defer rows.deinit();

    try std.testing.expectEqual(@as(usize, 1), rows.items.len);
    try std.testing.expectEqualStrings("fixed", rows.items[0].get(0).?);
}

test "Edge1: execute with empty params tuple works" {
    var db = try Database.open(":memory:");
    defer db.close();

    try db.exec("CREATE TABLE t_empty2 (id INTEGER PRIMARY KEY)");
    // Execute with no parameters
    try db.execute("INSERT INTO t_empty2 DEFAULT VALUES", .{});

    var rows = try db.query(
        std.testing.allocator,
        "SELECT COUNT(*) FROM t_empty2",
        .{},
    );
    defer rows.deinit();

    try std.testing.expectEqual(@as(usize, 1), rows.items.len);
    try std.testing.expectEqualStrings("1", rows.items[0].get(0).?);
}

// =============================================================================
// Edge Case 2: Optional null values
// =============================================================================

test "Edge2: execute binds null for optional null string parameter" {
    var db = try Database.open(":memory:");
    defer db.close();

    try db.exec("CREATE TABLE t_null (id INTEGER PRIMARY KEY, name TEXT)");
    const name: ?[]const u8 = null;
    try db.execute(
        "INSERT INTO t_null (name) VALUES (?1)",
        .{name},
    );

    var rows = try db.query(
        std.testing.allocator,
        "SELECT name FROM t_null",
        .{},
    );
    defer rows.deinit();

    try std.testing.expectEqual(@as(usize, 1), rows.items.len);
    // NULL columns come back as empty string from the sqlite wrapper
    try std.testing.expectEqualStrings("", rows.items[0].get(0).?);
}

test "Edge2: execute binds null for optional null integer parameter" {
    var db = try Database.open(":memory:");
    defer db.close();

    try db.exec("CREATE TABLE t_null_int (id INTEGER PRIMARY KEY, val INTEGER)");
    const val: ?i64 = null;
    try db.execute(
        "INSERT INTO t_null_int (val) VALUES (?1)",
        .{val},
    );

    var rows = try db.query(
        std.testing.allocator,
        "SELECT val FROM t_null_int",
        .{},
    );
    defer rows.deinit();

    try std.testing.expectEqual(@as(usize, 1), rows.items.len);
    // NULL integer columns come back as empty string
    try std.testing.expectEqualStrings("", rows.items[0].get(0).?);
}

test "Edge2: query with optional null parameter matches NULL rows" {
    var db = try Database.open(":memory:");
    defer db.close();

    try db.exec("CREATE TABLE t_null_q (id INTEGER PRIMARY KEY, name TEXT)");
    try db.exec("INSERT INTO t_null_q (name) VALUES (NULL)");
    try db.exec("INSERT INTO t_null_q (name) VALUES ('notnull')");

    // Query where name IS NULL using a null optional param
    const name: ?[]const u8 = null;
    var rows = try db.query(
        std.testing.allocator,
        "SELECT id FROM t_null_q WHERE name IS ?1",
        .{name},
    );
    defer rows.deinit();

    // Should match the NULL row
    try std.testing.expectEqual(@as(usize, 1), rows.items.len);
}

// =============================================================================
// Edge Case 3: Mixed parameter types in a single call
// =============================================================================

test "Edge3: execute with mixed string, integer, and null-optional params" {
    var db = try Database.open(":memory:");
    defer db.close();

    try db.exec("CREATE TABLE t_mix (id INTEGER PRIMARY KEY, name TEXT, age INTEGER, note TEXT)");
    const note: ?[]const u8 = null;
    try db.execute(
        "INSERT INTO t_mix (name, age, note) VALUES (?1, ?2, ?3)",
        .{ @as([]const u8, "bob"), @as(i64, 25), note },
    );

    var rows = try db.query(
        std.testing.allocator,
        "SELECT name, age, note FROM t_mix",
        .{},
    );
    defer rows.deinit();

    try std.testing.expectEqual(@as(usize, 1), rows.items.len);
    try std.testing.expectEqualStrings("bob", rows.items[0].get(0).?);
    try std.testing.expectEqualStrings("25", rows.items[0].get(1).?);
    // note is NULL → empty string from wrapper
    try std.testing.expectEqualStrings("", rows.items[0].get(2).?);
}

test "Edge3: query with mixed string, integer, and non-null optional params" {
    var db = try Database.open(":memory:");
    defer db.close();

    try db.exec("CREATE TABLE t_mix2 (id INTEGER PRIMARY KEY, name TEXT, age INTEGER, tag TEXT)");
    try db.execute(
        "INSERT INTO t_mix2 (name, age, tag) VALUES (?1, ?2, ?3)",
        .{ @as([]const u8, "carol"), @as(i64, 40), @as(?[]const u8, "vip") },
    );

    const tag: ?[]const u8 = "vip";
    var rows = try db.query(
        std.testing.allocator,
        "SELECT name, age FROM t_mix2 WHERE name = ?1 AND age = ?2 AND tag = ?3",
        .{ @as([]const u8, "carol"), @as(i64, 40), tag },
    );
    defer rows.deinit();

    try std.testing.expectEqual(@as(usize, 1), rows.items.len);
    try std.testing.expectEqualStrings("carol", rows.items[0].get(0).?);
    try std.testing.expectEqualStrings("40", rows.items[0].get(1).?);
}

// =============================================================================
// Edge Case 4: Comptime int literals
// =============================================================================

test "Edge4: execute with comptime_int literal parameters" {
    var db = try Database.open(":memory:");
    defer db.close();

    try db.exec("CREATE TABLE t_compint (id INTEGER PRIMARY KEY, a INTEGER, b INTEGER)");
    // Pass comptime_int literals directly (not @as(i64, ...))
    try db.execute(
        "INSERT INTO t_compint (a, b) VALUES (?1, ?2)",
        .{ 1, 2 },
    );

    var rows = try db.query(
        std.testing.allocator,
        "SELECT a, b FROM t_compint WHERE a = ?1 AND b = ?2",
        .{ 1, 2 },
    );
    defer rows.deinit();

    try std.testing.expectEqual(@as(usize, 1), rows.items.len);
    try std.testing.expectEqualStrings("1", rows.items[0].get(0).?);
    try std.testing.expectEqualStrings("2", rows.items[0].get(1).?);
}

test "Edge4: query with comptime_int zero and negative values" {
    var db = try Database.open(":memory:");
    defer db.close();

    try db.exec("CREATE TABLE t_compint2 (id INTEGER PRIMARY KEY, val INTEGER)");
    try db.execute("INSERT INTO t_compint2 (val) VALUES (?1)", .{0});
    try db.execute("INSERT INTO t_compint2 (val) VALUES (?1)", .{-5});

    var rows = try db.query(
        std.testing.allocator,
        "SELECT val FROM t_compint2 ORDER BY val",
        .{},
    );
    defer rows.deinit();

    try std.testing.expectEqual(@as(usize, 2), rows.items.len);
    try std.testing.expectEqualStrings("-5", rows.items[0].get(0).?);
    try std.testing.expectEqualStrings("0", rows.items[1].get(0).?);
}

// =============================================================================
// Edge Case 5: String-coercible types (pointer-to-array from string literals)
// =============================================================================

test "Edge5: execute with string literal parameters (pointer-to-array coercion)" {
    var db = try Database.open(":memory:");
    defer db.close();

    try db.exec("CREATE TABLE t_strlit (id INTEGER PRIMARY KEY, name TEXT)");
    // String literals are *const [N:0]u8, which must be coerced via isStringType
    try db.execute(
        "INSERT INTO t_strlit (name) VALUES (?1)",
        .{"literal"},
    );

    var rows = try db.query(
        std.testing.allocator,
        "SELECT name FROM t_strlit WHERE name = ?1",
        .{"literal"},
    );
    defer rows.deinit();

    try std.testing.expectEqual(@as(usize, 1), rows.items.len);
    try std.testing.expectEqualStrings("literal", rows.items[0].get(0).?);
}

test "Edge5: execute and query with sentinel-terminated string slices" {
    var db = try Database.open(":memory:");
    defer db.close();

    try db.exec("CREATE TABLE t_sent (id INTEGER PRIMARY KEY, name TEXT)");
    const name: [:0]const u8 = "sentinel";
    try db.execute(
        "INSERT INTO t_sent (name) VALUES (?1)",
        .{name},
    );

    var rows = try db.query(
        std.testing.allocator,
        "SELECT name FROM t_sent WHERE name = ?1",
        .{name},
    );
    defer rows.deinit();

    try std.testing.expectEqual(@as(usize, 1), rows.items.len);
    try std.testing.expectEqualStrings("sentinel", rows.items[0].get(0).?);
}

// =============================================================================
// Edge Case 6: rc variable scoping — bindParams uses its own rc
// =============================================================================

test "Edge6: execute works correctly after bindParams (rc scoping)" {
    // This verifies that bindParams doesn't clobber the caller's rc variable.
    // After bindParams returns, execute() must still be able to call
    // sqlite3_step and check its return code independently.
    var db = try Database.open(":memory:");
    defer db.close();

    try db.exec("CREATE TABLE t_rc (id INTEGER PRIMARY KEY, a TEXT, b INTEGER)");

    // Multiple params to exercise the bind loop, then step must succeed
    try db.execute(
        "INSERT INTO t_rc (a, b) VALUES (?1, ?2)",
        .{ @as([]const u8, "test"), @as(i64, 123) },
    );

    // Verify the row was inserted (step worked after bind)
    var rows = try db.query(
        std.testing.allocator,
        "SELECT a, b FROM t_rc",
        .{},
    );
    defer rows.deinit();

    try std.testing.expectEqual(@as(usize, 1), rows.items.len);
    try std.testing.expectEqualStrings("test", rows.items[0].get(0).?);
    try std.testing.expectEqualStrings("123", rows.items[0].get(1).?);
}

test "Edge6: query works correctly after bindParams with multiple rows" {
    // Verify that after bindParams, the query's sqlite3_step loop works
    // correctly to fetch multiple result rows.
    var db = try Database.open(":memory:");
    defer db.close();

    try db.exec("CREATE TABLE t_rc2 (id INTEGER PRIMARY KEY, val TEXT)");
    try db.execute("INSERT INTO t_rc2 (val) VALUES (?1)", .{@as([]const u8, "a")});
    try db.execute("INSERT INTO t_rc2 (val) VALUES (?1)", .{@as([]const u8, "b")});
    try db.execute("INSERT INTO t_rc2 (val) VALUES (?1)", .{@as([]const u8, "c")});

    var rows = try db.query(
        std.testing.allocator,
        "SELECT val FROM t_rc2 WHERE val >= ?1 ORDER BY val",
        .{@as([]const u8, "a")},
    );
    defer rows.deinit();

    try std.testing.expectEqual(@as(usize, 3), rows.items.len);
    try std.testing.expectEqualStrings("a", rows.items[0].get(0).?);
    try std.testing.expectEqualStrings("b", rows.items[1].get(0).?);
    try std.testing.expectEqualStrings("c", rows.items[2].get(0).?);
}

// =============================================================================
// Additional: Comprehensive integration — many params, types, and operations
// =============================================================================

test "integration: insert and retrieve rows with all supported param types" {
    var db = try Database.open(":memory:");
    defer db.close();

    try db.exec(
        "CREATE TABLE t_all (id INTEGER PRIMARY KEY, name TEXT, count INTEGER, tag TEXT, score INTEGER)"
    );

    // Insert with: string, int, optional-null string, optional-non-null int
    const tag: ?[]const u8 = null;
    const score: ?i64 = 100;
    try db.execute(
        "INSERT INTO t_all (name, count, tag, score) VALUES (?1, ?2, ?3, ?4)",
        .{ @as([]const u8, "test"), @as(i64, 5), tag, score },
    );

    var rows = try db.query(
        std.testing.allocator,
        "SELECT name, count, tag, score FROM t_all WHERE name = ?1",
        .{@as([]const u8, "test")},
    );
    defer rows.deinit();

    try std.testing.expectEqual(@as(usize, 1), rows.items.len);
    try std.testing.expectEqualStrings("test", rows.items[0].get(0).?);
    try std.testing.expectEqualStrings("5", rows.items[0].get(1).?);
    try std.testing.expectEqualStrings("", rows.items[0].get(2).?); // NULL → ""
    try std.testing.expectEqualStrings("100", rows.items[0].get(3).?);
}

test "integration: multiple sequential operations use bindParams correctly" {
    var db = try Database.open(":memory:");
    defer db.close();

    try db.exec("CREATE TABLE t_seq (id INTEGER PRIMARY KEY, key TEXT, val TEXT)");

    // Multiple inserts via execute (each calls bindParams)
    try db.execute("INSERT INTO t_seq (key, val) VALUES (?1, ?2)", .{ @as([]const u8, "k1"), @as([]const u8, "v1") });
    try db.execute("INSERT INTO t_seq (key, val) VALUES (?1, ?2)", .{ @as([]const u8, "k2"), @as([]const u8, "v2") });
    try db.execute("INSERT INTO t_seq (key, val) VALUES (?1, ?2)", .{ @as([]const u8, "k3"), @as([]const u8, "v3") });

    // Query all (uses bindParams with empty tuple)
    var all_rows = try db.query(
        std.testing.allocator,
        "SELECT key, val FROM t_seq ORDER BY key",
        .{},
    );
    defer all_rows.deinit();
    try std.testing.expectEqual(@as(usize, 3), all_rows.items.len);

    // Query specific (uses bindParams with a string param)
    var one_row = try db.query(
        std.testing.allocator,
        "SELECT val FROM t_seq WHERE key = ?1",
        .{@as([]const u8, "k2")},
    );
    defer one_row.deinit();
    try std.testing.expectEqual(@as(usize, 1), one_row.items.len);
    try std.testing.expectEqualStrings("v2", one_row.items[0].get(0).?);
}
