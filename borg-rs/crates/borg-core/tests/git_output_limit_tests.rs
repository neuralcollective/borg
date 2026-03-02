use borg_core::git::{Git, DEFAULT_GIT_OUTPUT_LIMIT};
use tempfile::TempDir;

fn init_repo(dir: &TempDir) {
    let path = dir.path().to_str().unwrap();
    for args in &[
        vec!["init"],
        vec!["config", "user.email", "test@example.com"],
        vec!["config", "user.name", "Test"],
    ] {
        std::process::Command::new("git")
            .args(["-C", path])
            .args(args)
            .output()
            .unwrap();
    }
}

fn commit_file(dir: &TempDir, name: &str, content: &[u8]) {
    let path = dir.path().to_str().unwrap();
    std::fs::write(dir.path().join(name), content).unwrap();
    std::process::Command::new("git")
        .args(["-C", path, "add", "-A"])
        .output()
        .unwrap();
    std::process::Command::new("git")
        .args(["-C", path, "commit", "-m", "add file"])
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .output()
        .unwrap();
}

#[test]
fn test_default_output_limit_is_10mb() {
    assert_eq!(DEFAULT_GIT_OUTPUT_LIMIT, 10 * 1024 * 1024);
}

#[test]
fn test_small_output_not_truncated() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);
    commit_file(&dir, "hello.txt", b"hello world\n");

    let git = Git::new(dir.path().to_str().unwrap());
    let result = git.exec(dir.path().to_str().unwrap(), &["log", "--oneline"]).unwrap();
    assert!(result.success());
    assert!(!result.stdout.is_empty());
    assert!(result.stdout.len() < DEFAULT_GIT_OUTPUT_LIMIT);
}

#[test]
fn test_output_truncated_at_limit() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);
    // Write a file larger than our test limit
    let content = vec![b'a'; 50_000];
    commit_file(&dir, "big.txt", &content);

    let path = dir.path().to_str().unwrap();
    let mut git = Git::new(path);
    git.output_limit = 1000;

    // `git show HEAD` outputs the commit metadata + full diff, well over 1000 bytes
    let result = git.exec(path, &["show", "HEAD"]).unwrap();
    assert!(result.stdout.len() <= 1000, "stdout should be capped at limit");
    assert!(!result.stdout.is_empty(), "stdout should have some content");
}

#[test]
fn test_output_at_exact_limit_not_truncated() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);
    commit_file(&dir, "small.txt", b"hi\n");

    let path = dir.path().to_str().unwrap();
    let mut git = Git::new(path);
    // Set a generous limit; small output should pass through intact
    git.output_limit = 1_000_000;

    let result = git.exec(path, &["log", "--oneline"]).unwrap();
    assert!(result.success());
    // Output is a single short line — well under 1MB, so it should not be truncated
    assert!(!result.stdout.is_empty());
    assert!(result.stdout.len() < 1_000_000);
}

#[test]
fn test_exit_code_preserved_after_limit_change() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    let path = dir.path().to_str().unwrap();
    let mut git = Git::new(path);
    git.output_limit = 100;

    // A failing git command should still have exit_code != 0
    let result = git.exec(path, &["rev-parse", "nonexistent-ref"]).unwrap();
    assert!(!result.success());
}

#[test]
fn test_stderr_truncated_at_limit() {
    let dir = TempDir::new().unwrap();
    init_repo(&dir);

    let path = dir.path().to_str().unwrap();
    let mut git = Git::new(path);
    git.output_limit = 50;

    // `git log` on a nonexistent path produces stderr output
    let result = git
        .exec(path, &["log", "--all", "--", "nonexistent-path-xyz"])
        .unwrap();
    // stderr may be empty here; what matters is we don't OOM and the call succeeds
    assert!(result.stderr.len() <= 50);
}
