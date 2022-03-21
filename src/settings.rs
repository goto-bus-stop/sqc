use rusqlite::{Connection, OptionalExtension};
use rusqlite::types::FromSqlError;
use std::path::Path;

const MIGRATIONS: [&str; 2] = [
    "PRAGMA application_id = 0xe170e644;",
    "CREATE TABLE datatypes (
        application_id INT NOT NULL,
        name TEXT UNIQUE NOT NULL,
        type TEXT NOT NULL
    );",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i64)]
pub enum DataTypeHint {
    Json,
}

pub struct Settings {
    conn: Connection,
}

impl Settings {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let conn = Connection::open(path)?;
        Ok(Self { conn })
    }

    pub fn get_data_type(&self, app_id: u32, column: &str) -> Option<DataTypeHint> {
        self.conn.query_row("SELECT type FROM datatypes WHERE application_id = ? AND name = ?", rusqlite::params![app_id, column], |row| {
            match row.get::<_, i64>(0)? {
                0 => Ok(DataTypeHint::Json),
                i => Err(FromSqlError::OutOfRange(i).into()),
            }
        })
        .optional()
        .unwrap_or_default()
    }
}
