use borg_core::sandbox::{Sandbox, SandboxMode};
use tempfile::TempDir;

#[test]
fn bwrap_parses_to_bwrap() {
    assert_eq!(
        SandboxMode::from_str_or_auto("bwrap"),
        Some(SandboxMode::Bwrap)
    );
}

#[test]
fn docker_parses_to_docker() {
    assert_eq!(
        SandboxMode::from_str_or_auto("docker"),
        Some(SandboxMode::Docker)
    );
}

#[test]
fn direct_parses_to_direct() {
    assert_eq!(
        SandboxMode::from_str_or_auto("direct"),
        Some(SandboxMode::Direct)
    );
}

#[test]
fn none_parses_to_direct() {
    assert_eq!(
        SandboxMode::from_str_or_auto("none"),
        Some(SandboxMode::Direct)
    );
}

#[test]
fn auto_returns_none() {
    assert_eq!(SandboxMode::from_str_or_auto("auto"), None);
}

#[test]
fn unknown_string_returns_none() {
    assert_eq!(SandboxMode::from_str_or_auto("podman"), None);
}

#[test]
fn empty_string_returns_none() {
    assert_eq!(SandboxMode::from_str_or_auto(""), None);
}

#[test]
fn case_insensitive_bwrap() {
    assert_eq!(
        SandboxMode::from_str_or_auto("BWRAP"),
        Some(SandboxMode::Bwrap)
    );
    assert_eq!(
        SandboxMode::from_str_or_auto("Bwrap"),
        Some(SandboxMode::Bwrap)
    );
}

#[test]
fn case_insensitive_docker() {
    assert_eq!(
        SandboxMode::from_str_or_auto("DOCKER"),
        Some(SandboxMode::Docker)
    );
    assert_eq!(
        SandboxMode::from_str_or_auto("Docker"),
        Some(SandboxMode::Docker)
    );
}

#[test]
fn case_insensitive_direct() {
    assert_eq!(
        SandboxMode::from_str_or_auto("DIRECT"),
        Some(SandboxMode::Direct)
    );
    assert_eq!(
        SandboxMode::from_str_or_auto("NONE"),
        Some(SandboxMode::Direct)
    );
}

fn strs(args: &[String]) -> Vec<&str> {
    args.iter().map(|s| s.as_str()).collect()
}

fn has_seq(haystack: &[&str], needle: &[&str]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

#[test]
fn bwrap_args_contains_unshare_all() {
    let args = Sandbox::bwrap_args(&[], "/work", &[]);
    assert!(
        strs(&args).contains(&"--unshare-all"),
        "args must contain --unshare-all for full namespace isolation"
    );
}

#[test]
fn bwrap_args_working_dir_set_via_chdir() {
    let args = Sandbox::bwrap_args(&[], "/some/working/dir", &[]);
    let s = strs(&args);
    assert!(
        has_seq(&s, &["--chdir", "/some/working/dir"]),
        "--chdir <working_dir> must be present in args"
    );
}

#[test]
fn bwrap_args_ro_bind_covers_slash() {
    let args = Sandbox::bwrap_args(&[], "/work", &[]);
    let s = strs(&args);
    assert!(
        has_seq(&s, &["--ro-bind", "/", "/"]),
        "--ro-bind / / must be present, covering /usr, /lib, and other read-only system paths"
    );
}

#[test]
fn bwrap_args_command_appended_after_separator() {
    let cmd = vec!["sh".to_string(), "-c".to_string(), "echo hello".to_string()];
    let args = Sandbox::bwrap_args(&[], "/work", &cmd);
    let s = strs(&args);
    let sep = s
        .iter()
        .position(|&a| a == "--")
        .expect("-- separator must be present");
    assert_eq!(&s[sep + 1..], &["sh", "-c", "echo hello"]);
}

#[test]
fn bwrap_args_zero_writable_dirs() {
    let cmd = vec!["echo".to_string(), "ok".to_string()];
    let args = Sandbox::bwrap_args(&[], "/tmp/work", &cmd);
    let s = strs(&args);

    assert!(s.contains(&"--unshare-all"));
    assert!(has_seq(&s, &["--ro-bind", "/", "/"]));
    assert!(s.contains(&"--"));
    assert!(s.contains(&"echo"));

    // /tmp uses --tmpfs, no extra --bind dirs
    assert!(has_seq(&s, &["--tmpfs", "/tmp"]));
    let bind_count = s.windows(3).filter(|w| w[0] == "--bind").count();
    assert_eq!(
        bind_count, 0,
        "zero writable dirs → no --bind entries"
    );
}

#[test]
fn bwrap_args_single_writable_dir_bound() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().to_str().unwrap();

    let args = Sandbox::bwrap_args(&[path], "/work", &[]);
    let s = strs(&args);

    assert!(
        has_seq(&s, &["--bind", path, path]),
        "--bind <dir> <dir> must appear for each writable directory"
    );
}

#[test]
fn bwrap_args_multiple_writable_dirs_all_bound() {
    let dir1 = TempDir::new().unwrap();
    let dir2 = TempDir::new().unwrap();
    let dir3 = TempDir::new().unwrap();
    let paths: Vec<&str> = [&dir1, &dir2, &dir3]
        .iter()
        .map(|d| d.path().to_str().unwrap())
        .collect();

    let cmd = vec!["cargo".to_string(), "test".to_string()];
    let args = Sandbox::bwrap_args(&paths, paths[0], &cmd);
    let s = strs(&args);

    for path in &paths {
        assert!(
            has_seq(&s, &["--bind", path, path]),
            "--bind {path} {path} must be present for each writable dir"
        );
    }

    // Command follows -- separator
    let sep = s
        .iter()
        .position(|&a| a == "--")
        .expect("-- separator must be present");
    assert_eq!(&s[sep + 1..], &["cargo", "test"]);
}

#[test]
fn bwrap_args_nonexistent_dir_skipped() {
    let path = "/nonexistent/borg/test/dir/that/cannot/exist";
    let args = Sandbox::bwrap_args(&[path], "/work", &[]);
    let s = strs(&args);

    // Non-existent dir must not appear in --bind args
    assert!(
        !has_seq(&s, &["--bind", path, path]),
        "non-existent writable dir must be silently skipped"
    );
}
