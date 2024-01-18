//! Additional functions for SQLite, especially for data display.

use humansize::{format_size_i, DECIMAL};
use rusqlite::{functions::FunctionFlags, Connection};

pub fn install(conn: &Connection) -> rusqlite::Result<()> {
    conn.create_scalar_function(
        "fmt_byte_size",
        1,
        FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
        |ctx| {
            let n: i64 = ctx.get(0)?;
            Ok(format_size_i(n, DECIMAL))
        },
    )?;

    Ok(())
}
