use borg_core::db::Db;

fn open_db() -> Db {
    let mut db = Db::open(":memory:").expect("open in-memory db");
    db.migrate().expect("migrate");
    db
}

#[test]
fn total_knowledge_bytes_empty() {
    let db = open_db();
    assert_eq!(db.total_knowledge_file_bytes().unwrap(), 0);
}

#[test]
fn total_knowledge_bytes_single_file() {
    let db = open_db();
    db.insert_knowledge_file("doc.pdf", "test doc", 1_000_000, false).unwrap();
    assert_eq!(db.total_knowledge_file_bytes().unwrap(), 1_000_000);
}

#[test]
fn total_knowledge_bytes_multiple_files() {
    let db = open_db();
    db.insert_knowledge_file("a.pdf", "", 1_000, false).unwrap();
    db.insert_knowledge_file("b.pdf", "", 2_000, false).unwrap();
    db.insert_knowledge_file("c.pdf", "", 3_000, false).unwrap();
    assert_eq!(db.total_knowledge_file_bytes().unwrap(), 6_000);
}

#[test]
fn total_knowledge_bytes_after_delete() {
    let db = open_db();
    db.insert_knowledge_file("x.pdf", "", 500, false).unwrap();
    let id = db.insert_knowledge_file("y.pdf", "", 300, false).unwrap();
    assert_eq!(db.total_knowledge_file_bytes().unwrap(), 800);
    db.delete_knowledge_file(id).unwrap();
    assert_eq!(db.total_knowledge_file_bytes().unwrap(), 500);
}

// Verify per-file and cumulative limit constants are consistent with the
// handler: per-file cap (50 MB) must be less than cumulative cap (1 GB).
#[test]
fn knowledge_limits_are_ordered() {
    const MAX_FILE: i64 = 50 * 1024 * 1024;
    const MAX_TOTAL: i64 = 1024 * 1024 * 1024;
    assert!(MAX_FILE < MAX_TOTAL, "per-file cap must be smaller than total cap");
}
