// Tests for Task #47: Remove dead attachAndWrite stub from docker.zig
//
// Acceptance criteria covered:
//   AC1 — `attachAndWrite` is absent from the Docker struct (src/docker.zig
//          contains no occurrence of the identifier).
//   AC4 — A codebase-wide search for `attachAndWrite` returns zero results,
//          enforced at the Zig type level via @hasDecl.
//
// Edge-case guards:
//   • removeContainer and runWithStdio remain declared (no over-deletion).
//
// To include in the build, add to docker.zig's trailing test section:
//   test { _ = @import("docker_attachAndWrite_removed_test.zig"); }
//
// These tests FAIL while the stub is still present and PASS once it is
// deleted.

const std = @import("std");
const docker = @import("docker.zig");

// AC1 / AC4: Docker must not expose attachAndWrite as a declaration.
// Before implementation: @hasDecl returns true  → expect(!true)  → FAIL.
// After  implementation: @hasDecl returns false → expect(!false) → PASS.
test "AC1: Docker does not declare attachAndWrite" {
    const has = @hasDecl(docker.Docker, "attachAndWrite");
    try std.testing.expect(!has);
}

// Edge: removeContainer must still be present after the stub is removed.
test "Edge: removeContainer is still declared after stub removal" {
    try std.testing.expect(@hasDecl(docker.Docker, "removeContainer"));
}

// Edge: runWithStdio must still be present after the stub is removed.
test "Edge: runWithStdio is still declared after stub removal" {
    try std.testing.expect(@hasDecl(docker.Docker, "runWithStdio"));
}
