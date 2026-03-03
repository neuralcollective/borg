use borg_core::db::Db;

fn open_db() -> Db {
    let mut db = Db::open(":memory:").expect("open in-memory db");
    db.migrate().expect("migrate");
    db
}

fn unit_vec(dims: usize) -> Vec<f32> {
    vec![1.0 / (dims as f32).sqrt(); dims]
}

#[test]
fn test_search_embeddings_returns_top_results() {
    let db = open_db();
    let high = vec![1.0f32, 0.0, 0.0, 0.0];
    let mid = vec![0.5f32, 0.5, 0.5, 0.5];
    let low = vec![0.0f32, 0.0, 0.0, 1.0];

    db.upsert_embedding(None, None, "high", "a.txt", &high).unwrap();
    db.upsert_embedding(None, None, "mid", "b.txt", &mid).unwrap();
    db.upsert_embedding(None, None, "low", "c.txt", &low).unwrap();

    let query = vec![1.0f32, 0.0, 0.0, 0.0];
    let results = db.search_embeddings(&query, 2, None).unwrap();

    assert_eq!(results.len(), 2);
    assert_eq!(results[0].chunk_text, "high");
    assert_eq!(results[1].chunk_text, "mid");
}

#[test]
fn test_search_embeddings_with_project_filter() {
    let db = open_db();
    let v = vec![1.0f32, 0.0];

    let pid1 = db.insert_project("p1", "swe", "/r", "", "", "", "").unwrap();
    let pid2 = db.insert_project("p2", "swe", "/r", "", "", "", "").unwrap();

    db.upsert_embedding(Some(pid1), None, "proj1", "a.txt", &v).unwrap();
    db.upsert_embedding(Some(pid2), None, "proj2", "b.txt", &v).unwrap();

    let results = db.search_embeddings(&v, 10, Some(pid1)).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].chunk_text, "proj1");
}

#[test]
fn test_search_embeddings_scan_cap_limits_rows_fetched() {
    let db = open_db();
    let dims = 2;
    let v = unit_vec(dims);

    // Insert more rows than SEARCH_SCAN_LIMIT
    let over = Db::SEARCH_SCAN_LIMIT + 50;
    for i in 0..over {
        let text = format!("chunk-{i}");
        // Give each chunk a unique hash by varying the text
        db.upsert_embedding(None, None, &text, "f.txt", &v).unwrap();
    }

    // Requesting more than SEARCH_SCAN_LIMIT results
    let results = db.search_embeddings(&v, over, None).unwrap();
    // Must not return more rows than the hard cap
    assert!(
        results.len() <= Db::SEARCH_SCAN_LIMIT,
        "returned {} rows, expected <= {}",
        results.len(),
        Db::SEARCH_SCAN_LIMIT
    );
}

#[test]
fn test_search_embeddings_no_rows() {
    let db = open_db();
    let v = vec![1.0f32, 0.0];
    let results = db.search_embeddings(&v, 5, None).unwrap();
    assert!(results.is_empty());
}
