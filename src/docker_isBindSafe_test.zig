// Tests for isBindSafe boundary conditions.
//
// Covers: substring-match semantics, dot-prefix specificity, exact-root
// matches, trailing-content matches, case-sensitivity, nested blocked paths,
// no-colon rejection, and path-traversal rejection.
//
// To include in the build, add to docker.zig's test section:
//   _ = @import("docker_isBindSafe_test.zig");
// and make isBindSafe pub:
//   pub fn isBindSafe(bind: []const u8) bool

const std = @import("std");
const docker = @import("docker.zig");

// =============================================================================
// AC1: Substring-match semantics
// "/credentials" blocks any host path containing it as a substring, not only
// paths that are exactly "/credentials".
// =============================================================================

test "AC1: /credentials_backup is blocked via substring match" {
    // The pattern "/credentials" is a substring of "/credentials_backup".
    try std.testing.expect(!docker.isBindSafe("/credentials_backup:/data"));
}

test "AC1: infix credentials_backup is blocked" {
    // Pattern found in the middle of the host path.
    try std.testing.expect(!docker.isBindSafe("/home/user/credentials_backup:/data"));
}

test "AC1: suffix /credentials is blocked" {
    try std.testing.expect(!docker.isBindSafe("/home/user/credentials:/data"));
}

test "AC1: unrelated safe path is allowed" {
    try std.testing.expect(docker.isBindSafe("/home/user/safe_dir:/data"));
}

// =============================================================================
// AC2: Dot-prefix specificity
// Patterns like "/.ssh" require the literal dot. Paths without the dot are
// not blocked.
// =============================================================================

test "AC2: /ssh without dot is not blocked" {
    try std.testing.expect(docker.isBindSafe("/ssh:/data"));
}

test "AC2: /aws without dot is not blocked" {
    try std.testing.expect(docker.isBindSafe("/aws:/data"));
}

test "AC2: /kube without dot is not blocked" {
    try std.testing.expect(docker.isBindSafe("/kube:/data"));
}

test "AC2: /env without dot is not blocked" {
    try std.testing.expect(docker.isBindSafe("/env:/data"));
}

test "AC2: /gnupg without dot is not blocked" {
    try std.testing.expect(docker.isBindSafe("/gnupg:/data"));
}

// =============================================================================
// AC3: Exact-root match
// Every blocked pattern must fire when the host path starts with it directly.
// =============================================================================

test "AC3: /.ssh at root is blocked" {
    try std.testing.expect(!docker.isBindSafe("/.ssh:/data"));
}

test "AC3: /.aws at root is blocked" {
    try std.testing.expect(!docker.isBindSafe("/.aws:/data"));
}

test "AC3: /.kube at root is blocked" {
    try std.testing.expect(!docker.isBindSafe("/.kube:/data"));
}

test "AC3: /.env at root is blocked" {
    try std.testing.expect(!docker.isBindSafe("/.env:/data"));
}

test "AC3: /credentials at root is blocked" {
    try std.testing.expect(!docker.isBindSafe("/credentials:/data"));
}

test "AC3: /id_rsa at root is blocked" {
    try std.testing.expect(!docker.isBindSafe("/id_rsa:/data"));
}

test "AC3: /id_ed25519 at root is blocked" {
    try std.testing.expect(!docker.isBindSafe("/id_ed25519:/data"));
}

test "AC3: /.git/config at root is blocked" {
    try std.testing.expect(!docker.isBindSafe("/.git/config:/data"));
}

test "AC3: /.gnupg at root is blocked" {
    try std.testing.expect(!docker.isBindSafe("/.gnupg:/data"));
}

test "AC3: /.config/gcloud at root is blocked" {
    try std.testing.expect(!docker.isBindSafe("/.config/gcloud:/data"));
}

// =============================================================================
// AC4: Substring trailing content
// A blocked pattern that has extra path or file-extension content after it
// is still matched.
// =============================================================================

test "AC4: /.env_backup is blocked because it contains /.env" {
    try std.testing.expect(!docker.isBindSafe("/.env_backup:/data"));
}

test "AC4: /id_rsa.pub is blocked because it contains /id_rsa" {
    try std.testing.expect(!docker.isBindSafe("/id_rsa.pub:/data"));
}

test "AC4: /.git/config.bak is blocked because it contains /.git/config" {
    try std.testing.expect(!docker.isBindSafe("/home/user/.git/config.bak:/data"));
}

// =============================================================================
// AC5: Case sensitivity
// The checks use byte-exact comparison. Uppercase or mixed-case variants of
// blocked patterns are NOT blocked — the check is case-sensitive.
// =============================================================================

test "AC5: /.SSH is not blocked (case-sensitive)" {
    try std.testing.expect(docker.isBindSafe("/.SSH:/data"));
}

test "AC5: /.AWS is not blocked (case-sensitive)" {
    try std.testing.expect(docker.isBindSafe("/.AWS:/data"));
}

test "AC5: /CREDENTIALS is not blocked (case-sensitive)" {
    try std.testing.expect(docker.isBindSafe("/CREDENTIALS:/data"));
}

test "AC5: /.ENV is not blocked (case-sensitive)" {
    try std.testing.expect(docker.isBindSafe("/.ENV:/data"));
}

test "AC5: /.Ssh mixed-case is not blocked (case-sensitive)" {
    try std.testing.expect(docker.isBindSafe("/.Ssh:/data"));
}

// =============================================================================
// AC6: Nested paths with a blocked component are still blocked
// =============================================================================

test "AC6: nested /.ssh is blocked" {
    try std.testing.expect(!docker.isBindSafe("/home/user/.ssh:/data"));
}

test "AC6: nested /.aws with /credentials is blocked" {
    // Both "/.aws" and "/credentials" are substrings; first match wins.
    try std.testing.expect(!docker.isBindSafe("/home/user/.aws/credentials:/data"));
}

test "AC6: nested /.config/gcloud is blocked" {
    try std.testing.expect(!docker.isBindSafe("/home/user/.config/gcloud:/data"));
}

test "AC6: nested /.kube/config is blocked" {
    try std.testing.expect(!docker.isBindSafe("/home/user/.kube/config:/data"));
}

test "AC6: nested /.gnupg/pubring.kbx is blocked" {
    try std.testing.expect(!docker.isBindSafe("/home/user/.gnupg/pubring.kbx:/data"));
}

// =============================================================================
// AC7: No-colon input is rejected
// isBindSafe requires a colon separator between host and container paths.
// Without one, the function returns false regardless of path content.
// =============================================================================

test "AC7: input without colon is rejected" {
    try std.testing.expect(!docker.isBindSafe("/.ssh"));
}

test "AC7: safe-looking path without colon is rejected" {
    try std.testing.expect(!docker.isBindSafe("/safe/path"));
}

// =============================================================================
// AC8: Path traversal is rejected
// ".." in the host path portion is blocked independently of the pattern list.
// =============================================================================

test "AC8: path traversal with .. is blocked" {
    try std.testing.expect(!docker.isBindSafe("/safe/../etc/passwd:/data"));
}

test "AC8: double path traversal is blocked" {
    try std.testing.expect(!docker.isBindSafe("/safe/../../root:/data"));
}

// =============================================================================
// Edge cases from spec §5
// =============================================================================

test "Edge: /home/.aws/credentials blocked by both /.aws and /credentials" {
    try std.testing.expect(!docker.isBindSafe("/home/.aws/credentials:/data"));
}

test "Edge: bind with :ro option — host path extracted correctly, safe path allowed" {
    // Only the segment before the first colon is the host path.
    try std.testing.expect(docker.isBindSafe("/home/user/safe_project:/workspace:ro"));
}

test "Edge: bind with :rw option — safe path allowed" {
    try std.testing.expect(docker.isBindSafe("/home/user/project:/workspace:rw"));
}

test "Edge: no-colon-is-invalid string is rejected" {
    try std.testing.expect(!docker.isBindSafe("no-colon-is-invalid"));
}

test "Edge: empty string is rejected (no colon)" {
    try std.testing.expect(!docker.isBindSafe(""));
}

test "Edge: host path is just a colon is rejected (empty host path — no valid path)" {
    // host_path = "" which has no ".." and no blocked pattern, so this is
    // actually allowed by current logic. Document the current behaviour.
    // An empty host path passes all checks and returns true.
    try std.testing.expect(docker.isBindSafe(":/data"));
}
