use borg_core::db::Db;
use std::sync::Mutex;

static DB_LOCK: Mutex<()> = Mutex::new(());

pub struct TestDb {
    pub db: Db,
    _guard: std::sync::MutexGuard<'static, ()>,
}

impl std::ops::Deref for TestDb {
    type Target = Db;
    fn deref(&self) -> &Db {
        &self.db
    }
}

pub fn open_db() -> TestDb {
    let guard = DB_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let database_url = std::env::var("TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap_or_else(|_| "postgres://borg:borg@127.0.0.1:5432/borg".to_string());
    let mut db = Db::open(&database_url).expect("open postgres test db");
    db.migrate().expect("migrate");
    TestDb { db, _guard: guard }
}
