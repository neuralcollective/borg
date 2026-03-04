// Tests for the CID-reading task JoinHandle fix.
//
// The CID-reading task in claude.rs was previously spawned with `tokio::spawn`
// but the JoinHandle was immediately dropped, causing panics inside it to be
// silently discarded. These tests verify the abort-then-await pattern that
// the fix relies on behaves correctly.

// A panicking spawned task must be observable through its JoinHandle.
#[tokio::test]
async fn test_panicking_task_visible_through_join_handle() {
    let handle: tokio::task::JoinHandle<()> = tokio::spawn(async {
        panic!("deliberate test panic");
    });
    // Allow the task to run and panic.
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    handle.abort();
    let result = handle.await;
    assert!(result.is_err());
    assert!(
        result.unwrap_err().is_panic(),
        "expected JoinError::panic, got cancellation"
    );
}

// A task that is aborted before it panics returns JoinError::cancelled.
#[tokio::test]
async fn test_aborted_task_returns_cancelled() {
    let handle: tokio::task::JoinHandle<()> = tokio::spawn(async {
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        panic!("should never reach here");
    });
    handle.abort();
    let result = handle.await;
    assert!(result.is_err());
    assert!(
        result.unwrap_err().is_cancelled(),
        "expected JoinError::cancelled, got panic"
    );
}

// A task that completes normally before abort returns Ok(()).
#[tokio::test]
async fn test_completed_task_returns_ok_after_abort() {
    let handle: tokio::task::JoinHandle<()> = tokio::spawn(async {
        // completes immediately
    });
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    handle.abort(); // no-op: already finished
    let result = handle.await;
    assert!(result.is_ok());
}
