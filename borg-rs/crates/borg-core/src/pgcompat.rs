use std::{
    cell::Cell,
    fmt,
    marker::PhantomData,
    sync::Arc,
};

use deadpool_postgres::{Manager, ManagerConfig, Pool, RecyclingMethod};
use tokio_postgres::{
    types::ToSql as PgToSql,
    NoTls,
    Row as PgRow,
};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    Postgres(tokio_postgres::Error),
    Pool(deadpool_postgres::PoolError),
    QueryReturnedNoRows,
    ConfigParse(String),
    SessionUnavailable(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Postgres(err) => write!(f, "postgres error: {err}"),
            Self::Pool(err) => write!(f, "postgres pool error: {err}"),
            Self::QueryReturnedNoRows => write!(f, "query returned no rows"),
            Self::ConfigParse(err) => write!(f, "postgres config parse error: {err}"),
            Self::SessionUnavailable(err) => write!(f, "postgres session unavailable: {err}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<tokio_postgres::Error> for Error {
    fn from(value: tokio_postgres::Error) -> Self {
        Self::Postgres(value)
    }
}

impl From<deadpool_postgres::PoolError> for Error {
    fn from(value: deadpool_postgres::PoolError) -> Self {
        Self::Pool(value)
    }
}

#[derive(Debug, Clone)]
pub enum Param {
    Text(Option<String>),
    Int8(Option<i64>),
    Int4(Option<i32>),
    Float8(Option<f64>),
    Bool(Option<bool>),
    Bytes(Option<Vec<u8>>),
}

impl Param {
    fn as_pg(&self) -> &(dyn PgToSql + Sync) {
        match self {
            Self::Text(value) => value,
            Self::Int8(value) => value,
            Self::Int4(value) => value,
            Self::Float8(value) => value,
            Self::Bool(value) => value,
            Self::Bytes(value) => value,
        }
    }
}

pub trait ToSql: Send + Sync {
    fn to_param(&self) -> Param;
}

pub mod types {
    pub use super::ToSql;
}

macro_rules! impl_to_sql_text {
    ($ty:ty) => {
        impl ToSql for $ty {
            fn to_param(&self) -> Param {
                Param::Text(Some(self.to_string()))
            }
        }
    };
}

impl_to_sql_text!(String);
impl_to_sql_text!(&str);
impl_to_sql_text!(&String);

impl ToSql for i64 {
    fn to_param(&self) -> Param {
        Param::Int8(Some(*self))
    }
}

impl ToSql for i32 {
    fn to_param(&self) -> Param {
        Param::Int4(Some(*self))
    }
}

impl ToSql for u32 {
    fn to_param(&self) -> Param {
        Param::Int8(Some(*self as i64))
    }
}

impl ToSql for usize {
    fn to_param(&self) -> Param {
        Param::Int8(Some(*self as i64))
    }
}

impl ToSql for bool {
    fn to_param(&self) -> Param {
        Param::Bool(Some(*self))
    }
}

impl ToSql for f64 {
    fn to_param(&self) -> Param {
        Param::Float8(Some(*self))
    }
}

impl ToSql for Vec<u8> {
    fn to_param(&self) -> Param {
        Param::Bytes(Some(self.clone()))
    }
}

impl ToSql for &Vec<u8> {
    fn to_param(&self) -> Param {
        Param::Bytes(Some((*self).clone()))
    }
}

impl ToSql for Option<String> {
    fn to_param(&self) -> Param {
        Param::Text(self.clone())
    }
}

impl ToSql for Option<&str> {
    fn to_param(&self) -> Param {
        Param::Text(self.map(|value| value.to_string()))
    }
}

impl ToSql for Option<&String> {
    fn to_param(&self) -> Param {
        Param::Text(self.map(|value| value.to_string()))
    }
}

impl ToSql for Option<i64> {
    fn to_param(&self) -> Param {
        Param::Int8(*self)
    }
}

impl ToSql for Option<i32> {
    fn to_param(&self) -> Param {
        Param::Int4(*self)
    }
}

impl ToSql for Option<bool> {
    fn to_param(&self) -> Param {
        Param::Bool(*self)
    }
}

impl ToSql for Option<Vec<u8>> {
    fn to_param(&self) -> Param {
        Param::Bytes(self.clone())
    }
}

impl ToSql for Option<f64> {
    fn to_param(&self) -> Param {
        Param::Float8(*self)
    }
}

pub fn to_param<T: ToSql + ?Sized>(value: &T) -> Param {
    value.to_param()
}

macro_rules! params {
    ($($value:expr),* $(,)?) => {
        vec![$($crate::pgcompat::to_param(&$value)),*]
    };
}
pub(crate) use params;

pub trait ParamsLike {
    fn into_params(self) -> Vec<Param>;
}

impl<const N: usize> ParamsLike for [Param; N] {
    fn into_params(self) -> Vec<Param> {
        self.into_iter().collect()
    }
}

impl ParamsLike for Vec<Param> {
    fn into_params(self) -> Vec<Param> {
        self
    }
}

impl ParamsLike for &[Param] {
    fn into_params(self) -> Vec<Param> {
        self.to_vec()
    }
}

impl ParamsLike for &[&dyn ToSql] {
    fn into_params(self) -> Vec<Param> {
        self.iter().map(|value| value.to_param()).collect()
    }
}

pub trait OptionalExtension<T> {
    fn optional(self) -> Result<Option<T>>;
}

impl<T> OptionalExtension<T> for Result<T> {
    fn optional(self) -> Result<Option<T>> {
        match self {
            Ok(value) => Ok(Some(value)),
            Err(Error::QueryReturnedNoRows) => Ok(None),
            Err(err) => Err(err),
        }
    }
}

pub struct Connection {
    pool: Arc<Pool>,
}

impl Connection {
    pub fn open(url: &str) -> Result<Self> {
        let config = url
            .parse::<tokio_postgres::Config>()
            .map_err(|err| Error::ConfigParse(err.to_string()))?;
        let max_size = std::env::var("DATABASE_POOL_MAX_SIZE")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(32)
            .max(4);
        let mgr_config = ManagerConfig {
            recycling_method: RecyclingMethod::Fast,
        };
        let manager = Manager::from_config(config, NoTls, mgr_config);
        let pool = Pool::builder(manager)
            .max_size(max_size)
            .build()
            .map_err(|err| Error::ConfigParse(err.to_string()))?;
        Ok(Self { pool: Arc::new(pool) })
    }

    fn guard(&self) -> ConnectionGuard {
        let client = match block_on(self.pool.get()) {
            Ok(client) => client,
            Err(err) => return ConnectionGuard::failed(err.to_string()),
        };
        if let Err(err) = block_on(client.batch_execute("SET TIME ZONE 'UTC'")) {
            return ConnectionGuard::failed(err.to_string());
        }
        ConnectionGuard::ready(client)
    }
}

type PoolClient = deadpool_postgres::Object;

pub struct Mutex<T> {
    inner: T,
}

impl<T> Mutex<T> {
    pub fn new(inner: T) -> Self {
        Self { inner }
    }
}

pub struct PoisonError<T> {
    inner: T,
}

impl<T> PoisonError<T> {
    pub fn into_inner(self) -> T {
        self.inner
    }
}

pub type LockResult<T> = std::result::Result<T, PoisonError<T>>;

impl Mutex<Connection> {
    pub fn lock(&self) -> LockResult<ConnectionGuard> {
        Ok(self.inner.guard())
    }
}

enum GuardState {
    Ready(Option<PoolClient>),
    Failed(String),
}

pub struct ConnectionGuard {
    state: GuardState,
}

impl ConnectionGuard {
    fn ready(client: PoolClient) -> Self {
        Self {
            state: GuardState::Ready(Some(client)),
        }
    }

    fn failed(message: String) -> Self {
        Self {
            state: GuardState::Failed(message),
        }
    }

    fn with_client<T>(&self, f: impl FnOnce(&deadpool_postgres::Client) -> Result<T>) -> Result<T> {
        match &self.state {
            GuardState::Ready(Some(client)) => f(client),
            GuardState::Ready(None) => {
                Err(Error::SessionUnavailable("postgres session already released".into()))
            }
            GuardState::Failed(message) => Err(Error::SessionUnavailable(message.clone())),
        }
    }

    pub fn execute_batch(&self, sql: &str) -> Result<()> {
        let translated = translate_sql(sql);
        self.with_client(|client| {
            block_on(client.batch_execute(&translated))?;
            Ok(())
        })
    }

    pub fn execute<P: ParamsLike>(&self, sql: &str, params: P) -> Result<usize> {
        let translated = translate_sql(sql);
        let params = params.into_params();
        let refs = pg_refs(&params);
        self.with_client(|client| {
            let changed = block_on(client.execute(&translated, &refs))? as usize;
            Ok(changed)
        })
    }

    pub fn execute_returning_id<P: ParamsLike>(&self, sql: &str, params: P) -> Result<i64> {
        let translated = translate_sql(sql);
        let returning_sql = append_returning_id(&translated);
        let params = params.into_params();
        let refs = pg_refs(&params);
        self.with_client(|client| {
            let row = block_on(client.query_one(&returning_sql, &refs))?;
            let id: i64 = row.get(0);
            Ok(id)
        })
    }

    pub fn query_row<P: ParamsLike, T>(
        &self,
        sql: &str,
        params: P,
        mapper: impl FnOnce(&Row<'_>) -> Result<T>,
    ) -> Result<T> {
        let translated = translate_sql(sql);
        let params = params.into_params();
        let refs = pg_refs(&params);
        let row = self.with_client(|client| {
            block_on(client.query_opt(&translated, &refs))
                .map_err(Error::from)?
                .ok_or(Error::QueryReturnedNoRows)
        })?;
        let row = Row::new(row);
        mapper(&row)
    }

    pub fn prepare<'a>(&'a self, sql: &str) -> Result<Statement<'a>> {
        Ok(Statement {
            guard: self,
            sql: sql.to_string(),
        })
    }

    pub fn transaction(&self) -> Result<Transaction<'_>> {
        self.with_client(|client| {
            block_on(client.batch_execute("BEGIN"))?;
            Ok(())
        })?;
        Ok(Transaction {
            guard: self,
            committed: Cell::new(false),
        })
    }

}

pub struct Statement<'a> {
    guard: &'a ConnectionGuard,
    sql: String,
}

impl<'a> Statement<'a> {
    pub fn query_map<P: ParamsLike, T, F>(&mut self, params: P, mut mapper: F) -> Result<MappedRows<T>>
    where
        F: FnMut(&Row<'_>) -> Result<T>,
    {
        let translated = translate_sql(&self.sql);
        let params = params.into_params();
        let refs = pg_refs(&params);
        let rows = self.guard.with_client(|client| {
            Ok(block_on(client.query(&translated, &refs))?)
        })?;
        let mapped = rows
            .into_iter()
            .map(|row| {
                let row = Row::new(row);
                mapper(&row)
            })
            .collect::<Vec<_>>();
        Ok(MappedRows {
            inner: mapped.into_iter(),
        })
    }
}

pub struct MappedRows<T> {
    inner: std::vec::IntoIter<Result<T>>,
}

impl<T> Iterator for MappedRows<T> {
    type Item = Result<T>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}

pub struct Transaction<'a> {
    guard: &'a ConnectionGuard,
    committed: Cell<bool>,
}

impl<'a> Transaction<'a> {
    pub fn execute<P: ParamsLike>(&self, sql: &str, params: P) -> Result<usize> {
        self.guard.execute(sql, params)
    }

    pub fn commit(self) -> Result<()> {
        self.guard.with_client(|client| {
            block_on(client.batch_execute("COMMIT"))?;
            Ok(())
        })?;
        self.committed.set(true);
        Ok(())
    }
}

impl Drop for Transaction<'_> {
    fn drop(&mut self) {
        if self.committed.get() {
            return;
        }
        let _ = self.guard.with_client(|client| {
            block_on(client.batch_execute("ROLLBACK"))?;
            Ok(())
        });
    }
}

pub struct Row<'a> {
    row: PgRow,
    _marker: PhantomData<&'a ()>,
}

impl<'a> Row<'a> {
    fn new(row: PgRow) -> Self {
        Self {
            row,
            _marker: PhantomData,
        }
    }

    pub fn get<I, T>(&self, idx: I) -> Result<T>
    where
        I: RowIndex,
        T: for<'b> postgres_types::FromSql<'b>,
    {
        self.row.try_get(idx.index()).map_err(Error::from)
    }
}

pub trait RowIndex {
    fn index(self) -> usize;
}

impl RowIndex for usize {
    fn index(self) -> usize {
        self
    }
}

impl RowIndex for i32 {
    fn index(self) -> usize {
        self as usize
    }
}

impl RowIndex for u32 {
    fn index(self) -> usize {
        self as usize
    }
}

fn block_on<F: std::future::Future>(f: F) -> F::Output {
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        tokio::task::block_in_place(|| handle.block_on(f))
    } else {
        tokio::runtime::Runtime::new()
            .expect("failed to create tokio runtime for blocking pg call")
            .block_on(f)
    }
}

fn pg_refs(params: &[Param]) -> Vec<&(dyn PgToSql + Sync)> {
    params.iter().map(Param::as_pg).collect()
}

fn append_returning_id(sql: &str) -> String {
    let trimmed = sql.trim_end().trim_end_matches(';');
    format!("{trimmed} RETURNING id")
}

fn translate_sql(sql: &str) -> String {
    replace_placeholders(sql)
}

fn replace_placeholders(sql: &str) -> String {
    let bytes = sql.as_bytes();
    let mut out = String::with_capacity(sql.len() + 8);
    let mut i = 0;
    let mut next_anon = 1usize;
    while i < bytes.len() {
        match bytes[i] {
            b'\'' => {
                out.push('\'');
                i += 1;
                while i < bytes.len() {
                    out.push(bytes[i] as char);
                    if bytes[i] == b'\'' {
                        if i + 1 < bytes.len() && bytes[i + 1] == b'\'' {
                            i += 1;
                            out.push('\'');
                        } else {
                            i += 1;
                            break;
                        }
                    }
                    i += 1;
                }
            },
            b'?' => {
                i += 1;
                let start = i;
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
                out.push('$');
                if i > start {
                    out.push_str(&sql[start..i]);
                } else {
                    out.push_str(&next_anon.to_string());
                    next_anon += 1;
                }
            },
            byte => {
                out.push(byte as char);
                i += 1;
            },
        }
    }
    out
}
