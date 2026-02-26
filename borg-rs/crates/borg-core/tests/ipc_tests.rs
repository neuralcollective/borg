// Integration tests for borg_core::ipc — all ACs from spec.md.
// These tests reference borg_core::ipc which does not exist yet;
// they will fail to compile until the implementation is added.

use std::fs;
use std::io::{Seek, SeekFrom, Write};
use std::os::unix::fs as unix_fs;

use tempfile::TempDir;

use borg_core::ipc::{self, IpcReadResult, MAX_IPC_FILE_BYTES};

// ── helpers ──────────────────────────────────────────────────────────────────

fn write_file(dir: &TempDir, name: &str, content: &[u8]) {
    let path = dir.path().join(name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&path, content).unwrap();
}

fn errors_dir(dir: &TempDir) -> std::path::PathBuf {
    dir.path().join("errors")
}

fn errors_entry_count(dir: &TempDir) -> usize {
    let p = errors_dir(dir);
    if !p.exists() {
        return 0;
    }
    fs::read_dir(p).unwrap().count()
}

fn base(dir: &TempDir) -> String {
    dir.path().to_str().unwrap().to_string()
}

// ── AC1: validate_filename ────────────────────────────────────────────────────

#[test]
fn validate_dotdot_absolute_rejected() {
    // "../../etc/passwd" must be rejected
    assert!(ipc::validate_filename("../../etc/passwd").is_err());
}

#[test]
fn validate_absolute_path_rejected() {
    assert!(ipc::validate_filename("/abs/path").is_err());
}

#[test]
fn validate_dotdot_component_rejected() {
    assert!(ipc::validate_filename("../sibling").is_err());
}

#[test]
fn validate_dotdot_embedded_rejected() {
    assert!(ipc::validate_filename("a/../../etc/passwd").is_err());
}

#[test]
fn validate_subdirectory_accepted() {
    // ".borg/prompt.md" — one level of subdirectory, no ".." → Ok
    assert!(ipc::validate_filename(".borg/prompt.md").is_ok());
}

#[test]
fn validate_plain_name_accepted() {
    assert!(ipc::validate_filename("spec.md").is_ok());
    assert!(ipc::validate_filename("audit.md").is_ok());
    assert!(ipc::validate_filename("candidates.md").is_ok());
}

#[test]
fn validate_empty_string_rejected() {
    assert!(ipc::validate_filename("").is_err());
}

// ── AC2: symlink rejection ────────────────────────────────────────────────────

#[test]
fn symlink_to_regular_file_is_quarantined() {
    let dir = TempDir::new().unwrap();
    // Create the real file somewhere else
    let real = dir.path().join("real.txt");
    fs::write(&real, b"secret").unwrap();

    // Create a symlink in the base dir pointing at the real file
    let link = dir.path().join("spec.md");
    unix_fs::symlink(&real, &link).unwrap();

    let result = ipc::read_file(&base(&dir), "spec.md");
    assert!(
        matches!(result, IpcReadResult::Quarantined(_)),
        "symlink to regular file must be quarantined, got non-Quarantined"
    );

    // The symlink must no longer be at its original location
    assert!(
        !link.exists() && !link.symlink_metadata().is_ok(),
        "original symlink should have been moved to errors/"
    );
}

#[test]
fn symlink_quarantine_lands_in_errors_dir() {
    let dir = TempDir::new().unwrap();
    let real = dir.path().join("target.txt");
    fs::write(&real, b"data").unwrap();
    unix_fs::symlink(&real, dir.path().join("spec.md")).unwrap();

    let before = errors_entry_count(&dir);
    let _ = ipc::read_file(&base(&dir), "spec.md");
    let after = errors_entry_count(&dir);

    assert!(after > before, "errors/ should have gained an entry after quarantine");
}

// ── AC3: TOCTOU / O_NOFOLLOW ─────────────────────────────────────────────────
//
// We cannot inject a race, but we verify that a symlink created *at* the target
// path is always rejected regardless of how it was created, because O_NOFOLLOW
// makes the open(2) fail even if lstat raced.

#[test]
fn toctou_symlink_always_rejected() {
    let dir = TempDir::new().unwrap();
    let real = dir.path().join("real.bin");
    fs::write(&real, b"payload").unwrap();

    // Simulate: lstat might have seen a regular file, but at open time
    // it's a symlink.  We just place the symlink directly.
    unix_fs::symlink(&real, dir.path().join("artifact.md")).unwrap();

    let result = ipc::read_file(&base(&dir), "artifact.md");
    assert!(
        matches!(result, IpcReadResult::Quarantined(_)),
        "O_NOFOLLOW must reject symlink even when lstat pre-check could be bypassed by race"
    );
}

// ── AC4: size cap ────────────────────────────────────────────────────────────

#[test]
fn file_exactly_at_cap_is_accepted() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("big.md");
    let mut f = fs::File::create(&path).unwrap();
    // Write a sparse file of exactly MAX_IPC_FILE_BYTES bytes
    f.seek(SeekFrom::Start(MAX_IPC_FILE_BYTES - 1)).unwrap();
    f.write_all(b"\0").unwrap();
    drop(f);

    let result = ipc::read_file(&base(&dir), "big.md");
    assert!(
        matches!(result, IpcReadResult::Ok(_)),
        "file at exactly MAX_IPC_FILE_BYTES should be accepted"
    );
}

#[test]
fn file_one_byte_over_cap_is_quarantined() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("oversized.md");
    let mut f = fs::File::create(&path).unwrap();
    f.seek(SeekFrom::Start(MAX_IPC_FILE_BYTES)).unwrap();
    f.write_all(b"\0").unwrap();
    drop(f);

    let result = ipc::read_file(&base(&dir), "oversized.md");
    assert!(
        matches!(result, IpcReadResult::Quarantined(_)),
        "file one byte over MAX_IPC_FILE_BYTES must be quarantined"
    );
    assert!(errors_entry_count(&dir) > 0, "oversize file must appear in errors/");
}

// ── AC5: non-regular files ────────────────────────────────────────────────────

#[test]
fn directory_at_artifact_path_is_quarantined() {
    let dir = TempDir::new().unwrap();
    fs::create_dir(dir.path().join("not_a_file.md")).unwrap();

    let result = ipc::read_file(&base(&dir), "not_a_file.md");
    assert!(
        matches!(result, IpcReadResult::Quarantined(_)),
        "directory at artifact path must be quarantined"
    );
}

#[cfg(unix)]
#[test]
fn named_pipe_at_artifact_path_is_quarantined() {
    use std::process::Command;

    let dir = TempDir::new().unwrap();
    let fifo = dir.path().join("pipe.md");
    let status = Command::new("mkfifo").arg(&fifo).status().unwrap();
    assert!(status.success(), "mkfifo must succeed");

    let result = ipc::read_file(&base(&dir), "pipe.md");
    assert!(
        matches!(result, IpcReadResult::Quarantined(_)),
        "named pipe at artifact path must be quarantined"
    );
}

// ── AC6: quarantine directory creation ───────────────────────────────────────

#[test]
fn errors_dir_is_created_on_first_quarantine() {
    let dir = TempDir::new().unwrap();
    assert!(!errors_dir(&dir).exists(), "precondition: errors/ should not exist");

    // Trigger a quarantine via a symlink
    let real = dir.path().join("r.txt");
    fs::write(&real, b"x").unwrap();
    unix_fs::symlink(&real, dir.path().join("spec.md")).unwrap();
    let _ = ipc::read_file(&base(&dir), "spec.md");

    assert!(errors_dir(&dir).is_dir(), "errors/ directory should be created after quarantine");
}

#[test]
fn quarantined_file_has_timestamped_name() {
    let dir = TempDir::new().unwrap();
    let real = dir.path().join("r.txt");
    fs::write(&real, b"x").unwrap();
    unix_fs::symlink(&real, dir.path().join("spec.md")).unwrap();
    let _ = ipc::read_file(&base(&dir), "spec.md");

    let entry = fs::read_dir(errors_dir(&dir))
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    let fname = entry.file_name();
    let name = fname.to_string_lossy();
    // Expected pattern: "spec.md.<timestamp>" or "spec.md.<timestamp>.<counter>"
    assert!(name.starts_with("spec.md."), "quarantined file name should start with 'spec.md.', got {name}");
}

// ── AC6 edge: errors/ is itself a symlink ────────────────────────────────────

#[test]
fn errors_dir_that_is_symlink_does_not_cause_panic() {
    let dir = TempDir::new().unwrap();
    // Create a decoy directory and symlink errors/ → it
    let decoy = dir.path().join("decoy_errors");
    fs::create_dir(&decoy).unwrap();
    unix_fs::symlink(&decoy, errors_dir(&dir)).unwrap();

    let real = dir.path().join("r.txt");
    fs::write(&real, b"data").unwrap();
    unix_fs::symlink(&real, dir.path().join("art.md")).unwrap();

    // Must not panic; quarantine may be skipped but bad file should still be removed
    let result = ipc::read_file(&base(&dir), "art.md");
    assert!(
        matches!(result, IpcReadResult::Quarantined(_)),
        "still must return Quarantined even when errors/ is a symlink"
    );
}

// ── AC7: happy path ───────────────────────────────────────────────────────────

#[test]
fn regular_file_under_cap_returned_as_ok() {
    let dir = TempDir::new().unwrap();
    write_file(&dir, "spec.md", b"# Spec\n\nHello world\n");

    let result = ipc::read_file(&base(&dir), "spec.md");
    match result {
        IpcReadResult::Ok(contents) => {
            assert_eq!(contents, "# Spec\n\nHello world\n");
        }
        other => panic!("expected Ok, got {:?}", other),
    }
}

#[test]
fn subdirectory_file_returned_as_ok() {
    let dir = TempDir::new().unwrap();
    write_file(&dir, ".borg/prompt.md", b"project context\n");

    let result = ipc::read_file(&base(&dir), ".borg/prompt.md");
    assert!(matches!(result, IpcReadResult::Ok(_)), "subdirectory file should be accepted");
}

// ── AC8: check_artifact ───────────────────────────────────────────────────────

#[test]
fn check_artifact_returns_true_for_regular_file() {
    let dir = TempDir::new().unwrap();
    write_file(&dir, "spec.md", b"content");
    assert!(ipc::check_artifact(&base(&dir), "spec.md"));
}

#[test]
fn check_artifact_returns_false_for_absent_file() {
    let dir = TempDir::new().unwrap();
    assert!(!ipc::check_artifact(&base(&dir), "spec.md"));
}

#[test]
fn check_artifact_returns_false_for_symlink_and_quarantines() {
    let dir = TempDir::new().unwrap();
    let real = dir.path().join("real.md");
    fs::write(&real, b"content").unwrap();
    unix_fs::symlink(&real, dir.path().join("spec.md")).unwrap();

    // symlink must not be followed — check_artifact returns false
    assert!(
        !ipc::check_artifact(&base(&dir), "spec.md"),
        "check_artifact must return false for a symlinked artifact"
    );
    // and it must have been quarantined
    assert!(errors_entry_count(&dir) > 0, "symlinked artifact must be quarantined");
}

#[test]
fn check_artifact_returns_false_for_traversal_name() {
    let dir = TempDir::new().unwrap();
    // Even if the attacker writes "../../evil", validate_filename rejects it
    assert!(!ipc::check_artifact(&base(&dir), "../../evil"));
}

// ── AC9: NotFound / Quarantined fall-through ─────────────────────────────────

#[test]
fn read_file_not_found_for_absent_file() {
    let dir = TempDir::new().unwrap();
    assert!(matches!(ipc::read_file(&base(&dir), "missing.md"), IpcReadResult::NotFound));
}

#[test]
fn read_file_not_found_when_base_dir_missing() {
    let result = ipc::read_file("/nonexistent/path/that/cannot/exist", "spec.md");
    assert!(matches!(result, IpcReadResult::NotFound));
}

// ── AC: non-UTF-8 content is quarantined ─────────────────────────────────────

#[test]
fn non_utf8_file_is_quarantined() {
    let dir = TempDir::new().unwrap();
    // Write invalid UTF-8 bytes
    write_file(&dir, "binary.md", b"\xff\xfe invalid utf-8 \x80\x81");

    let result = ipc::read_file(&base(&dir), "binary.md");
    assert!(
        matches!(result, IpcReadResult::Quarantined(_)),
        "non-UTF-8 content must be quarantined, got non-Quarantined"
    );
}

// ── AC: read_trusted_path ─────────────────────────────────────────────────────

#[test]
fn read_trusted_path_accepts_regular_file() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("prompt.md");
    fs::write(&path, b"trusted content\n").unwrap();

    let result = ipc::read_trusted_path(path.to_str().unwrap());
    assert!(matches!(result, IpcReadResult::Ok(_)));
}

#[test]
fn read_trusted_path_rejects_symlink() {
    let dir = TempDir::new().unwrap();
    let real = dir.path().join("real.md");
    fs::write(&real, b"data").unwrap();
    let link = dir.path().join("link.md");
    unix_fs::symlink(&real, &link).unwrap();

    let result = ipc::read_trusted_path(link.to_str().unwrap());
    assert!(
        matches!(result, IpcReadResult::Quarantined(_)),
        "read_trusted_path must reject symlinks"
    );
}

#[test]
fn read_trusted_path_rejects_oversized_file() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("huge.md");
    let mut f = fs::File::create(&path).unwrap();
    f.seek(SeekFrom::Start(MAX_IPC_FILE_BYTES)).unwrap();
    f.write_all(b"\0").unwrap();
    drop(f);

    let result = ipc::read_trusted_path(path.to_str().unwrap());
    assert!(
        matches!(result, IpcReadResult::Quarantined(_)),
        "read_trusted_path must quarantine oversized files"
    );
}

#[test]
fn read_trusted_path_not_found_for_missing() {
    let result = ipc::read_trusted_path("/tmp/nonexistent_borg_ipc_test_12345.md");
    assert!(matches!(result, IpcReadResult::NotFound));
}

// ── edge: concurrent quarantine name collision ────────────────────────────────

#[test]
fn concurrent_quarantine_same_name_no_panic() {
    // Simulate two files being quarantined with the same base name in rapid succession.
    // The implementation must not panic; it may use a counter suffix.
    let dir = TempDir::new().unwrap();

    for _ in 0..3 {
        // Re-create the symlink each time since the previous call moved it
        let real = dir.path().join("real.txt");
        fs::write(&real, b"x").unwrap();
        let link = dir.path().join("spec.md");
        if !link.exists() && link.symlink_metadata().is_err() {
            unix_fs::symlink(&real, &link).unwrap();
        }
        let _ = ipc::read_file(&base(&dir), "spec.md");
    }

    // All entries should be in errors/ without panic
    assert!(errors_entry_count(&dir) >= 1);
}

// ── IpcReadResult must implement Debug ───────────────────────────────────────

#[test]
fn ipc_read_result_implements_debug() {
    // Just ensure Debug is derived — the panic message in other tests uses it
    let _ok: IpcReadResult = IpcReadResult::Ok("x".to_string());
    let _nf: IpcReadResult = IpcReadResult::NotFound;
    let _q: IpcReadResult = IpcReadResult::Quarantined("reason".to_string());
    // If IpcReadResult doesn't derive Debug this file won't compile
    println!("{:?}", IpcReadResult::NotFound);
}
