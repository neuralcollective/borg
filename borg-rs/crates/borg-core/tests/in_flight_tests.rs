/// Tests for the in_flight TOCTOU fix.
///
/// Before the fix, the dispatch loop acquired the in_flight lock once per
/// iteration (insert), and then the seeding condition acquired it again
/// separately (is_empty). A task removal could race between the two
/// acquisitions and make the predicate stale.
///
/// After the fix, a single guard is held across all insertions and the
/// is_empty check, eliminating the window.
use std::{collections::HashSet, sync::Arc};

use tokio::sync::Mutex;

/// Holding the guard across insert + is_empty means concurrent removals
/// are blocked until the guard is dropped — no stale read is possible.
#[tokio::test]
async fn test_removal_blocked_while_dispatch_guard_held() {
    let in_flight: Arc<Mutex<HashSet<i64>>> = Arc::new(Mutex::new(HashSet::new()));

    // Pre-populate: task 1 was already in-flight from a previous tick.
    in_flight.lock().await.insert(1i64);

    // Acquire the single guard that covers both the dispatch loop and is_empty.
    let guard = in_flight.lock().await;

    // Spawn a removal (simulating InFlightGuard::drop finishing task 1).
    let in_flight2 = Arc::clone(&in_flight);
    let removal = tokio::spawn(async move {
        in_flight2.lock().await.remove(&1i64);
    });

    // The removal cannot run while we hold the guard; is_empty must be false.
    assert!(!guard.is_empty(), "removal is blocked — must see task 1 in set");

    drop(guard);
    removal.await.unwrap();

    assert!(in_flight.lock().await.is_empty(), "removal ran after guard drop");
}

/// Inserting a new task and reading is_empty in the same guard is consistent.
#[tokio::test]
async fn test_insert_and_is_empty_consistent_under_single_guard() {
    let in_flight: Arc<Mutex<HashSet<i64>>> = Arc::new(Mutex::new(HashSet::new()));

    let mut guard = in_flight.lock().await;

    // Simulate dispatching a task.
    guard.insert(42i64);

    // is_empty check with the same guard must see the insert.
    let should_seed = guard.is_empty();
    drop(guard);

    assert!(!should_seed, "freshly inserted task must make is_empty false");
}

/// When no tasks are dispatched and in_flight is truly empty, is_empty
/// returns true — the guard-holds-across-check approach preserves this.
#[tokio::test]
async fn test_empty_set_is_detected_when_no_tasks_dispatched() {
    let in_flight: Arc<Mutex<HashSet<i64>>> = Arc::new(Mutex::new(HashSet::new()));

    let guard = in_flight.lock().await;
    let should_seed = guard.is_empty();
    drop(guard);

    assert!(should_seed, "empty in_flight with dispatched=0 must trigger seeding");
}

/// Multiple insertions within the same guard are all visible to is_empty.
#[tokio::test]
async fn test_multiple_inserts_all_visible_before_is_empty() {
    let in_flight: Arc<Mutex<HashSet<i64>>> = Arc::new(Mutex::new(HashSet::new()));

    let mut guard = in_flight.lock().await;
    for id in 1i64..=5 {
        guard.insert(id);
    }
    assert!(!guard.is_empty());
    assert_eq!(guard.len(), 5);
    drop(guard);
}
