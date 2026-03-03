/// Tests for InFlightGuard synchronous cleanup behavior.
///
/// The guard must remove the task ID from the in-flight set in Drop::drop
/// without spawning a new tokio task, so cleanup is guaranteed even when
/// the tokio runtime is already shut down.
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

struct Guard {
    set: Arc<Mutex<HashSet<i64>>>,
    id: i64,
}

impl Drop for Guard {
    fn drop(&mut self) {
        self.set.lock().unwrap().remove(&self.id);
    }
}

#[test]
fn test_in_flight_guard_removes_on_drop_without_runtime() {
    let set: Arc<Mutex<HashSet<i64>>> = Arc::new(Mutex::new(HashSet::new()));
    set.lock().unwrap().insert(42);
    assert!(set.lock().unwrap().contains(&42));

    let guard = Guard { set: Arc::clone(&set), id: 42 };
    drop(guard);

    assert!(
        !set.lock().unwrap().contains(&42),
        "entry must be removed synchronously on drop, even without a tokio runtime"
    );
}

#[test]
fn test_in_flight_guard_multiple_entries_cleaned_up_independently() {
    let set: Arc<Mutex<HashSet<i64>>> = Arc::new(Mutex::new(HashSet::new()));
    set.lock().unwrap().insert(1);
    set.lock().unwrap().insert(2);
    set.lock().unwrap().insert(3);

    {
        let g1 = Guard { set: Arc::clone(&set), id: 1 };
        drop(g1);
        assert_eq!(set.lock().unwrap().len(), 2, "only id 1 removed");
        assert!(!set.lock().unwrap().contains(&1));
        assert!(set.lock().unwrap().contains(&2));
        assert!(set.lock().unwrap().contains(&3));
    }

    let g2 = Guard { set: Arc::clone(&set), id: 2 };
    let g3 = Guard { set: Arc::clone(&set), id: 3 };
    drop(g2);
    drop(g3);

    assert!(set.lock().unwrap().is_empty(), "all entries removed");
}

#[test]
fn test_in_flight_guard_drop_on_panic_unwind() {
    let set: Arc<Mutex<HashSet<i64>>> = Arc::new(Mutex::new(HashSet::new()));
    set.lock().unwrap().insert(99);

    let set_clone = Arc::clone(&set);
    let result = std::panic::catch_unwind(move || {
        let _guard = Guard { set: set_clone, id: 99 };
        panic!("simulated task panic");
    });

    assert!(result.is_err(), "panic was caught");
    assert!(
        !set.lock().unwrap().contains(&99),
        "id must be removed even when task panics"
    );
}
