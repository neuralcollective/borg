use borg_core::git::Git;
use std::process::Command;

fn git_cmd(dir: &str, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .status()
        .expect("git command failed to spawn");
    assert!(status.success(), "git {} failed", args.join(" "));
}

fn make_clean_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().to_str().unwrap();

    git_cmd(path, &["init"]);
    git_cmd(path, &["config", "user.email", "test@test.com"]);
    git_cmd(path, &["config", "user.name", "Test"]);

    // Create an initial commit so the working tree is clean.
    std::fs::write(dir.path().join("README"), "init").unwrap();
    git_cmd(path, &["add", "README"]);
    git_cmd(path, &["commit", "-m", "init"]);

    dir
}

#[test]
fn test_commit_all_returns_false_on_clean_tree() {
    let dir = make_clean_repo();
    let path = dir.path().to_str().unwrap();

    let git = Git::new(path);
    let result = git.commit_all(path, "should not commit", None);

    assert!(result.is_ok(), "commit_all returned error: {:?}", result);
    assert_eq!(result.unwrap(), false, "expected Ok(false) on clean tree");
}
