use borg_core::db::Db;

pub fn open_db() -> Db {
    let database_url = std::env::var("TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap_or_else(|_| "postgres://borg:borg@127.0.0.1:5432/borg".to_string());
    let mut db = Db::open(&database_url).expect("open postgres test db");
    db.migrate().expect("migrate");
    db
}
