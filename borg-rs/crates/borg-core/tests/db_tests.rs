use borg_core::db::Db;
use tempfile::NamedTempFile;

#[test]
fn busy_timeout_is_set_on_open() {
    let tmp = NamedTempFile::new().unwrap();
    let db = Db::open(tmp.path().to_str().unwrap()).unwrap();
    let conn = db.raw_conn().lock().unwrap();
    let timeout: i64 = conn
        .query_row("PRAGMA busy_timeout", [], |row: &rusqlite::Row| row.get(0))
        .unwrap();
    assert_eq!(timeout, 5000);
}
