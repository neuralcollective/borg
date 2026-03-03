use borg_core::sandbox::Sandbox;
use tempfile::TempDir;

fn cmd(args: &[&str]) -> Vec<String> {
    args.iter().map(|s| s.to_string()).collect()
}

fn pos(args: &[String], token: &str) -> Option<usize> {
    args.iter().position(|a| a == token)
}

#[test]
fn ro_bind_root_and_dev_present_and_ordered() {
    let args = Sandbox::bwrap_args(&[], "/work", &cmd(&["sh"]));

    let ro = pos(&args, "--ro-bind").expect("--ro-bind missing");
    assert_eq!(args[ro + 1], "/", "--ro-bind src");
    assert_eq!(args[ro + 2], "/", "--ro-bind dst");

    let dev = pos(&args, "--dev").expect("--dev missing");
    assert_eq!(args[dev + 1], "/dev", "--dev path");

    assert!(ro < dev, "--ro-bind must precede --dev");
}

#[test]
fn command_appended_after_double_dash() {
    let command = cmd(&["claude", "--dangerously-skip-permissions"]);
    let args = Sandbox::bwrap_args(&[], "/work", &command);

    let sep = pos(&args, "--").expect("-- separator missing");
    let tail: Vec<&str> = args[sep + 1..].iter().map(String::as_str).collect();
    assert_eq!(tail, ["claude", "--dangerously-skip-permissions"]);
}

#[test]
fn nonexistent_writable_dir_omitted() {
    let missing = "/tmp/borg_sandbox_test_nonexistent_dir_abc123";
    let args = Sandbox::bwrap_args(&[missing], "/work", &cmd(&["true"]));

    // No --bind referencing the missing path should appear
    let has_bind = args
        .windows(3)
        .any(|w| w[0] == "--bind" && w[1] == missing);
    assert!(!has_bind, "non-existent dir must not appear as --bind");
}

#[test]
fn existing_writable_dir_bound_readwrite() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().to_str().unwrap();

    let args = Sandbox::bwrap_args(&[path], "/work", &cmd(&["true"]));

    let bound = args
        .windows(3)
        .any(|w| w[0] == "--bind" && w[1] == path && w[2] == path);
    assert!(bound, "--bind <dir> <dir> must appear for existing writable dir");
}

#[test]
fn chdir_set_to_working_dir() {
    let args = Sandbox::bwrap_args(&[], "/my/work/dir", &cmd(&["make"]));

    let idx = pos(&args, "--chdir").expect("--chdir missing");
    assert_eq!(args[idx + 1], "/my/work/dir");
}

#[test]
fn isolation_flags_present() {
    let args = Sandbox::bwrap_args(&[], "/work", &cmd(&["sh"]));
    for flag in ["--unshare-pid", "--new-session", "--die-with-parent"] {
        assert!(pos(&args, flag).is_some(), "{flag} missing from bwrap args");
    }
    let proc = pos(&args, "--proc").expect("--proc missing");
    assert_eq!(args[proc + 1], "/proc");
}

#[test]
fn chdir_precedes_double_dash() {
    let args = Sandbox::bwrap_args(&[], "/work", &cmd(&["sh"]));
    let chdir = pos(&args, "--chdir").unwrap();
    let sep = pos(&args, "--").unwrap();
    assert!(chdir < sep, "--chdir must come before --");
}

#[test]
fn tmp_always_bound() {
    // /tmp bind must appear even with no writable_dirs
    let args = Sandbox::bwrap_args(&[], "/work", &cmd(&["sh"]));
    let bound = args
        .windows(3)
        .any(|w| w[0] == "--bind" && w[1] == "/tmp" && w[2] == "/tmp");
    assert!(bound, "--bind /tmp /tmp must always be present");
}

#[test]
fn ro_bind_before_writable_bind() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().to_str().unwrap();
    let args = Sandbox::bwrap_args(&[path], "/work", &cmd(&["true"]));

    let ro = pos(&args, "--ro-bind").unwrap();
    let bind_idx = args
        .windows(3)
        .position(|w| w[0] == "--bind" && w[1] == path)
        .expect("writable bind missing");
    assert!(ro < bind_idx, "--ro-bind / / must come before --bind <dir>");
}
