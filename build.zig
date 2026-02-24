const std = @import("std");

pub fn build(b: *std.Build) void {
    const target = b.standardTargetOptions(.{});
    const optimize = b.standardOptimizeOption(.{});

    // SQLite C library
    const sqlite_lib = b.addStaticLibrary(.{
        .name = "sqlite3",
        .target = target,
        .optimize = .ReleaseFast,
    });
    sqlite_lib.addCSourceFile(.{
        .file = b.path("vendor/sqlite/sqlite3.c"),
        .flags = &.{
            "-DSQLITE_THREADSAFE=1",
            "-DSQLITE_ENABLE_FTS5",
            "-DSQLITE_ENABLE_JSON1",
        },
    });
    sqlite_lib.addIncludePath(b.path("vendor/sqlite"));
    sqlite_lib.linkLibC();

    const exe_mod = b.createModule(.{
        .root_source_file = b.path("src/main.zig"),
        .target = target,
        .optimize = optimize,
    });
    exe_mod.addIncludePath(b.path("vendor/sqlite"));
    exe_mod.linkLibrary(sqlite_lib);

    const exe = b.addExecutable(.{
        .name = "borg",
        .root_module = exe_mod,
    });
    exe.linkLibC();
    b.installArtifact(exe);

    const run_cmd = b.addRunArtifact(exe);
    run_cmd.step.dependOn(b.getInstallStep());
    if (b.args) |args| {
        run_cmd.addArgs(args);
    }
    const run_step = b.step("run", "Run borg orchestrator");
    run_step.dependOn(&run_cmd.step);

    const exe_unit_tests = b.addTest(.{
        .root_module = exe_mod,
    });
    exe_unit_tests.linkLibC();
    const run_tests = b.addRunArtifact(exe_unit_tests);
    const test_step = b.step("test", "Run unit tests");
    test_step.dependOn(&run_tests.step);
}
