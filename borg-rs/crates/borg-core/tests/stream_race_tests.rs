// Tests for the stream-end / subscribe race condition.
//
// Invariants verified:
// 1. A late subscriber (after end_task) always sees stream_end in history
//    and receives no live receiver.
// 2. An early subscriber (before end_task) receives stream_end via the
//    broadcast channel; it never hangs waiting for a message that will never arrive.
// 3. A concurrent subscribe + end_task always yields stream_end through
//    exactly one of the two paths — no hang.
// 4. Pushing MAX_HISTORY_LINES messages before ending still preserves
//    stream_end as the terminal entry in history.

use std::{sync::Arc, time::Duration};

use borg_core::stream::TaskStreamManager;

// ---------------------------------------------------------------------------
// 1. Late subscriber sees stream_end in history, no live receiver
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_late_subscriber_sees_stream_end_in_history() {
    let mgr = TaskStreamManager::new();
    let id: i64 = 1001;
    mgr.start(id).await;
    mgr.push_line(id, "line1".to_string()).await;
    mgr.end_task(id).await;

    let (history, rx) = mgr.subscribe(id).await;

    assert!(rx.is_none(), "ended stream must return no live receiver");
    assert!(
        history.iter().any(|l| l.contains("stream_end")),
        "ended stream must have stream_end in history: {history:?}"
    );
}

// ---------------------------------------------------------------------------
// 2. Early subscriber receives stream_end via broadcast (no hang)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_early_subscriber_receives_stream_end_via_broadcast() {
    let mgr = TaskStreamManager::new();
    let id: i64 = 1002;
    mgr.start(id).await;
    mgr.push_line(id, "lineA".to_string()).await;

    let (history, rx) = mgr.subscribe(id).await;
    assert!(rx.is_some(), "live stream must return a receiver");
    assert!(
        !history.iter().any(|l| l.contains("stream_end")),
        "stream_end must not be in history before end_task"
    );

    let mut rx = rx.unwrap();

    // End the stream after subscribing.
    mgr.end_task(id).await;

    let msg = tokio::time::timeout(Duration::from_millis(200), rx.recv())
        .await
        .expect("timed out waiting for stream_end — receiver hung")
        .expect("broadcast recv error");

    assert!(
        msg.contains("stream_end"),
        "broadcast must deliver stream_end; got: {msg}"
    );
}

// ---------------------------------------------------------------------------
// 3. Concurrent subscribe + end_task: stream_end always reachable, no hang
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_concurrent_subscribe_and_end_no_hang() {
    let mgr = Arc::new(TaskStreamManager::new());
    let id: i64 = 1003;
    mgr.start(id).await;

    let mgr_end = mgr.clone();
    let end_handle = tokio::spawn(async move {
        mgr_end.end_task(id).await;
    });

    // Subscribe concurrently with end_task.
    let (history, rx) = mgr.subscribe(id).await;
    end_handle.await.unwrap();

    if let Some(mut rx) = rx {
        // Subscriber arrived before end; receiver must deliver stream_end.
        let msg = tokio::time::timeout(Duration::from_millis(500), rx.recv())
            .await
            .expect("timed out — subscriber hung without receiving stream_end")
            .expect("recv error");
        assert!(
            msg.contains("stream_end"),
            "expected stream_end; got: {msg}"
        );
    } else {
        // Subscriber arrived after end; history must contain stream_end.
        assert!(
            history.iter().any(|l| l.contains("stream_end")),
            "late-arriving subscriber: history must contain stream_end; got: {history:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// 4. Stream_end survives history truncation (>= MAX_HISTORY_LINES pushes)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_stream_end_survives_history_overflow() {
    let mgr = TaskStreamManager::new();
    let id: i64 = 1004;
    mgr.start(id).await;

    // Push enough lines to fill and overflow the history ring buffer.
    // MAX_HISTORY_LINES is 10_000; we push 10_001 to guarantee at least one eviction.
    for i in 0..10_001u32 {
        mgr.push_line(id, format!("line-{i}")).await;
    }
    mgr.end_task(id).await;

    let (history, rx) = mgr.subscribe(id).await;
    assert!(rx.is_none(), "ended stream must return no live receiver");
    assert!(
        history.iter().any(|l| l.contains("stream_end")),
        "stream_end must survive history overflow; history len={}",
        history.len()
    );
    assert_eq!(
        history.last().map(|s| s.as_str()),
        Some(r#"{"type":"stream_end"}"#),
        "stream_end must be the last entry in history after overflow"
    );
}

// ---------------------------------------------------------------------------
// 5. Ended stream: ended flag set means history invariant holds
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_ended_flag_implies_stream_end_in_history() {
    let mgr = TaskStreamManager::new();
    let id: i64 = 1005;
    mgr.start(id).await;
    mgr.end_task(id).await;

    // Multiple subscribe calls all yield the same terminal history.
    for _ in 0..3 {
        let (history, rx) = mgr.subscribe(id).await;
        assert!(rx.is_none());
        assert!(
            history.iter().any(|l| l.contains("stream_end")),
            "every subscribe on an ended stream must see stream_end in history"
        );
    }
}

// ---------------------------------------------------------------------------
// 6. Live receiver delivers all messages in order including stream_end
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_live_receiver_delivers_messages_in_order() {
    let mgr = TaskStreamManager::new();
    let id: i64 = 1006;
    mgr.start(id).await;

    let (_history, rx) = mgr.subscribe(id).await;
    let mut rx = rx.expect("must have live receiver");

    mgr.push_line(id, "msg1".to_string()).await;
    mgr.push_line(id, "msg2".to_string()).await;
    mgr.end_task(id).await;

    let recv = |rx: &mut tokio::sync::broadcast::Receiver<String>| {
        let r = rx.try_recv();
        r
    };

    let m1 = recv(&mut rx).expect("msg1");
    let m2 = recv(&mut rx).expect("msg2");
    let end = recv(&mut rx).expect("stream_end");

    assert_eq!(m1, "msg1");
    assert_eq!(m2, "msg2");
    assert!(
        end.contains("stream_end"),
        "last message must be stream_end"
    );
}
