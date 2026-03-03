use borg_core::db::Db;

fn open_db() -> Db {
    let mut db = Db::open(":memory:").expect("open in-memory db");
    db.migrate().expect("migrate");
    db
}

#[test]
fn test_insert_and_retrieve_stored_path() {
    let db = open_db();
    let id = db
        .insert_knowledge_file("report.pdf", "1234_abc_report.pdf", "annual report", 100, false)
        .expect("insert");
    let file = db.get_knowledge_file(id).expect("get").expect("exists");
    assert_eq!(file.file_name, "report.pdf");
    assert_eq!(file.stored_path, "1234_abc_report.pdf");
    assert_eq!(file.description, "annual report");
    assert!(!file.inline);
}

#[test]
fn test_same_filename_gets_unique_stored_paths() {
    let db = open_db();
    let id1 = db
        .insert_knowledge_file("policy.txt", "111_aaa_policy.txt", "", 10, false)
        .expect("insert first");
    let id2 = db
        .insert_knowledge_file("policy.txt", "222_bbb_policy.txt", "", 20, false)
        .expect("insert second");

    let f1 = db.get_knowledge_file(id1).expect("get").expect("exists");
    let f2 = db.get_knowledge_file(id2).expect("get").expect("exists");

    assert_eq!(f1.file_name, "policy.txt");
    assert_eq!(f2.file_name, "policy.txt");
    assert_ne!(f1.stored_path, f2.stored_path);
    assert_eq!(f1.stored_path, "111_aaa_policy.txt");
    assert_eq!(f2.stored_path, "222_bbb_policy.txt");
}

#[test]
fn test_list_knowledge_files_includes_stored_path() {
    let db = open_db();
    db.insert_knowledge_file("a.txt", "ts1_a.txt", "desc a", 5, true)
        .expect("insert a");
    db.insert_knowledge_file("b.txt", "ts2_b.txt", "desc b", 7, false)
        .expect("insert b");

    let files = db.list_knowledge_files().expect("list");
    assert_eq!(files.len(), 2);
    assert_eq!(files[0].stored_path, "ts1_a.txt");
    assert_eq!(files[1].stored_path, "ts2_b.txt");
}

#[test]
fn test_backfill_sets_stored_path_from_file_name() {
    // Simulate a row inserted with empty stored_path (as if it came from a pre-migration DB).
    // We test that migrate() back-fills stored_path = file_name for such rows.
    let mut db = Db::open(":memory:").expect("open db");
    db.migrate().expect("first migrate");

    // Insert a row with stored_path explicitly set to '' to simulate old data.
    {
        let conn = db.raw_conn().lock().unwrap();
        conn.execute(
            "INSERT INTO knowledge_files (file_name, stored_path, description, size_bytes, inline) VALUES ('old.txt', '', 'old file', 42, 0)",
            [],
        ).expect("direct insert");
    }

    // Re-run migrate() — the back-fill UPDATE should set stored_path = file_name.
    db.migrate().expect("second migrate");

    let files = db.list_knowledge_files().expect("list");
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].file_name, "old.txt");
    assert_eq!(files[0].stored_path, "old.txt");
}
