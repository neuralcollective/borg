use borg_core::db::Db;

fn open_db() -> Db {
    let mut db = Db::open(":memory:").expect("open in-memory db");
    db.migrate().expect("migrate");
    db
}

#[test]
fn get_config_str_returns_none_for_missing_key() {
    let db = open_db();
    assert_eq!(db.get_config_str("nonexistent"), None);
}

#[test]
fn get_config_str_returns_value_after_set() {
    let db = open_db();
    db.set_config("model", "claude-opus-4-6").unwrap();
    assert_eq!(db.get_config_str("model").as_deref(), Some("claude-opus-4-6"));
}

#[test]
fn get_config_str_returns_updated_value() {
    let db = open_db();
    db.set_config("model", "sonnet").unwrap();
    db.set_config("model", "opus").unwrap();
    assert_eq!(db.get_config_str("model").as_deref(), Some("opus"));
}

#[test]
fn get_config_str_matches_get_config_ok_flatten() {
    let db = open_db();
    db.set_config("focus", "finish the task").unwrap();
    let via_str = db.get_config_str("focus");
    let via_chain = db.get_config("focus").ok().flatten();
    assert_eq!(via_str, via_chain);
}

#[test]
fn get_config_str_missing_matches_get_config_ok_flatten() {
    let db = open_db();
    let via_str = db.get_config_str("missing_key");
    let via_chain = db.get_config("missing_key").ok().flatten();
    assert_eq!(via_str, via_chain);
}
