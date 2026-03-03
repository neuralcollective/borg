use borg_core::sandbox::Sandbox;

#[test]
fn branch_hash_is_deterministic() {
    assert_eq!(Sandbox::branch_hash("main"), Sandbox::branch_hash("main"));
    assert_ne!(Sandbox::branch_hash("main"), Sandbox::branch_hash("feature/foo"));
}

#[test]
fn branch_hash_length_is_eight() {
    assert_eq!(Sandbox::branch_hash("main").len(), 8);
    assert_eq!(Sandbox::branch_hash("a-very-long-branch-name-that-exceeds-normal-length").len(), 8);
}

#[test]
fn branch_volume_name_format() {
    let name = Sandbox::branch_volume_name("myrepo", "main", "target");
    assert!(name.starts_with("borg-cache-myrepo-"));
    assert!(name.ends_with("-target"));
}

#[test]
fn main_volume_name_matches_main_branch() {
    let main_vol = Sandbox::main_volume_name("myrepo", "cargo");
    let branch_vol = Sandbox::branch_volume_name("myrepo", "main", "cargo");
    assert_eq!(main_vol, branch_vol);
}

#[test]
fn bwrap_args_contains_ro_bind_root() {
    let args = Sandbox::bwrap_args(&[], "/work", &["echo".to_string(), "hi".to_string()]);
    let joined = args.join(" ");
    assert!(joined.contains("--ro-bind / /"));
    assert!(joined.contains("--chdir /work"));
    assert!(joined.contains("echo hi"));
}

#[test]
fn bwrap_args_includes_writable_dirs_that_exist() {
    let tmp = std::env::temp_dir();
    let tmp_str = tmp.to_str().unwrap();
    let args = Sandbox::bwrap_args(&[tmp_str], "/work", &["ls".to_string()]);
    let joined = args.join(" ");
    assert!(joined.contains(&format!("--bind {tmp_str} {tmp_str}")));
}

#[test]
fn bwrap_args_skips_nonexistent_writable_dirs() {
    let args = Sandbox::bwrap_args(&["/nonexistent/path/xyz"], "/work", &["ls".to_string()]);
    let joined = args.join(" ");
    assert!(!joined.contains("/nonexistent/path/xyz"));
}

// Verifies kill_agent_containers queries ALL containers (no status filter),
// unlike prune_orphan_containers which only touches exited/dead/created ones.
// Requires Docker — run with: cargo test -- --ignored
#[tokio::test]
#[ignore]
async fn kill_agent_containers_removes_running_containers() {
    // Start a borg-agent labelled container
    let start = tokio::process::Command::new("docker")
        .args(["run", "-d", "--label", "borg-agent=1", "--rm", "busybox", "sleep", "60"])
        .output()
        .await
        .expect("docker run failed");
    assert!(start.status.success(), "failed to start test container");
    let id = String::from_utf8_lossy(&start.stdout).trim().to_string();
    assert!(!id.is_empty());

    // kill_agent_containers must stop and remove it
    Sandbox::kill_agent_containers().await;

    // Container should no longer exist
    let inspect = tokio::process::Command::new("docker")
        .args(["inspect", &id])
        .output()
        .await
        .unwrap();
    assert!(!inspect.status.success(), "container should have been removed");
}
