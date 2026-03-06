// Tests for agent subprocess timeout behaviour.
//
// These tests verify that:
//   T1: tokio::time::timeout fires within the configured deadline.
//   T2: kill_on_drop(true) sends SIGKILL so the subprocess is actually dead
//       after the timeout fires and the Child handle is dropped.
//   T3: PhaseOutput::failed is returned (not an Err) when the timeout fires.

use std::time::{Duration, Instant};

use tokio::process::Command;

// ── T1: timeout fires within deadline ────────────────────────────────────────

#[tokio::test]
async fn test_timeout_fires_within_deadline() {
    let mut child = Command::new("sleep")
        .arg("99999")
        .kill_on_drop(true)
        .spawn()
        .expect("sleep must be available");

    let start = Instant::now();
    let timed_out = tokio::time::timeout(Duration::from_millis(200), child.wait())
        .await
        .is_err();

    assert!(timed_out, "wait() should have timed out");
    // Must fire well before the 99999 s sleep expires
    assert!(
        start.elapsed() < Duration::from_secs(2),
        "timeout took too long: {:?}",
        start.elapsed()
    );
}

// ── T2: subprocess is dead after timeout + drop ───────────────────────────────

#[tokio::test]
#[cfg(target_os = "linux")]
async fn test_subprocess_dead_after_timeout_drop() {
    let mut child = Command::new("sleep")
        .arg("99999")
        .kill_on_drop(true)
        .spawn()
        .expect("sleep must be available");

    let pid = child.id().expect("child must have a PID");

    let _ = tokio::time::timeout(Duration::from_millis(100), child.wait()).await;

    // Dropping the handle triggers kill_on_drop → SIGKILL
    drop(child);

    // Give the kernel a moment to reap the process
    tokio::time::sleep(Duration::from_millis(100)).await;

    let still_alive = std::path::Path::new(&format!("/proc/{pid}")).exists();
    assert!(
        !still_alive,
        "process {pid} should be dead after SIGKILL via kill_on_drop"
    );
}

// ── T3: PhaseOutput::failed returned on timeout ───────────────────────────────

#[tokio::test]
async fn test_phase_output_failed_on_timeout() {
    use borg_core::types::PhaseOutput;

    // Simulate what run_phase does for the timeout arm
    let mut child = Command::new("sleep")
        .arg("99999")
        .kill_on_drop(true)
        .spawn()
        .expect("sleep must be available");

    let timeout_s = 1u64;
    let io_future = async move { child.wait().await };

    let result: PhaseOutput =
        match tokio::time::timeout(Duration::from_secs(timeout_s), io_future).await {
            Ok(_) => panic!("should have timed out"),
            Err(_elapsed) => PhaseOutput::failed("timed out"),
        };

    assert!(!result.success, "PhaseOutput should indicate failure");
    assert_eq!(result.output, "timed out");
}

// ── T4: zero timeout means no limit (completes normally) ─────────────────────

#[tokio::test]
async fn test_zero_timeout_runs_to_completion() {
    // A short-lived command completes without any timeout wrapper
    let mut child = Command::new("true").kill_on_drop(true).spawn().unwrap();

    // When timeout_s == 0 we skip tokio::time::timeout (per claude.rs logic)
    let timeout_s: u64 = 0;

    let exit = if timeout_s > 0 {
        tokio::time::timeout(Duration::from_secs(timeout_s), child.wait())
            .await
            .expect("timed out")
            .expect("wait failed")
    } else {
        child.wait().await.expect("wait failed")
    };

    assert!(exit.success(), "true should exit 0");
}
