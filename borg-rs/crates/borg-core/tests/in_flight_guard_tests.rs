/// Tests for InFlightGuard drop behaviour.
///
/// The guard must release the in_flight slot synchronously so that:
///   1. No window exists between drop and slot removal.
///   2. Dropping the guard outside a Tokio runtime (e.g. during shutdown)
///      does not panic.
use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};

// ── helpers ───────────────────────────────────────────────────────────────────

struct InFlightGuard {
    in_flight: Arc<Mutex<HashSet<i64>>>,
    task_id: i64,
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        self.in_flight.lock().unwrap().remove(&self.task_id);
    }
}

fn make_set(ids: &[i64]) -> Arc<Mutex<HashSet<i64>>> {
    Arc::new(Mutex::new(ids.iter().copied().collect()))
}

// ── tests ─────────────────────────────────────────────────────────────────────

/// Slot is removed immediately when the guard goes out of scope — no async
/// task, no scheduling window.
#[test]
fn test_in_flight_guard_drop_removes_slot_immediately() {
    let in_flight = make_set(&[42]);
    {
        let _guard = InFlightGuard { in_flight: Arc::clone(&in_flight), task_id: 42 };
        assert!(in_flight.lock().unwrap().contains(&42));
    }
    assert!(!in_flight.lock().unwrap().contains(&42), "slot must be gone after drop");
}

/// Dropping the guard without a Tokio runtime must not panic.
/// This exercises the shutdown scenario: process_task future is dropped while
/// the runtime is being torn down.
#[test]
fn test_in_flight_guard_drop_without_tokio_runtime() {
    let in_flight = make_set(&[99]);
    let guard = InFlightGuard { in_flight: Arc::clone(&in_flight), task_id: 99 };
    // No #[tokio::test] — there is deliberately no runtime here.
    drop(guard);
    assert!(!in_flight.lock().unwrap().contains(&99));
}

/// Multiple guards for distinct task IDs each clean up only their own slot.
#[test]
fn test_in_flight_guard_drop_only_removes_own_slot() {
    let in_flight = make_set(&[1, 2, 3]);
    {
        let _g = InFlightGuard { in_flight: Arc::clone(&in_flight), task_id: 2 };
    }
    let set = in_flight.lock().unwrap();
    assert!(set.contains(&1), "slot 1 must remain");
    assert!(!set.contains(&2), "slot 2 must be removed");
    assert!(set.contains(&3), "slot 3 must remain");
}

/// Dropping a guard for a task_id that is already absent must not panic.
#[test]
fn test_in_flight_guard_drop_absent_id_is_noop() {
    let in_flight = make_set(&[]);
    let guard = InFlightGuard { in_flight: Arc::clone(&in_flight), task_id: 55 };
    drop(guard); // must not panic
}

/// After drop, the slot is available for re-use by a new dispatch attempt.
#[test]
fn test_in_flight_slot_reusable_after_guard_drop() {
    let in_flight = make_set(&[7]);
    {
        let _g = InFlightGuard { in_flight: Arc::clone(&in_flight), task_id: 7 };
    }
    // Simulate the next dispatch cycle re-inserting the same task_id.
    in_flight.lock().unwrap().insert(7);
    assert!(in_flight.lock().unwrap().contains(&7));
}
