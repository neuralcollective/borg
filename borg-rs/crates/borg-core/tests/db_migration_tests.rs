use borg_core::db::Db;

#[test]
fn test_migrate_is_idempotent() {
    let mut db = Db::open(":memory:").expect("open in-memory db");
    db.migrate().expect("first migrate");
    db.migrate().expect("second migrate must succeed (duplicate columns silently ignored)");
}

#[test]
fn test_migrate_fresh_db_succeeds() {
    let mut db = Db::open(":memory:").expect("open in-memory db");
    db.migrate().expect("migrate fresh db");
}
