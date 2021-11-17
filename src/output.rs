use crate::highlight::SQLHighlighter;
use comfy_table::{Cell, Color, ContentArrangement, Table};
use itertools::Itertools;
use rusqlite::types::ValueRef;
use rusqlite::{Column, Row};
use std::io::Write;
use std::process::{Command, Stdio};

pub trait OutputRows {
    fn begin(&mut self, columns: &[Column<'_>]) -> anyhow::Result<()>;
    fn add_row(&mut self, row: &Row<'_>) -> anyhow::Result<()>;
    fn finish(&mut self) -> anyhow::Result<()>;
}

pub struct NullOutput;
impl OutputRows for NullOutput {
    fn begin(&mut self, _columns: &[Column<'_>]) -> anyhow::Result<()> {
        Ok(())
    }
    fn add_row(&mut self, _row: &Row<'_>) -> anyhow::Result<()> {
        Ok(())
    }
    fn finish(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
}

#[derive(Default)]
pub struct TableOutput {
    table: Option<Table>,
    num_columns: usize,
}

fn value_to_cell(value: ValueRef) -> Cell {
    match value {
        ValueRef::Null => Cell::new("NULL").fg(Color::DarkGrey),
        ValueRef::Integer(n) => Cell::new(n).fg(Color::Yellow),
        ValueRef::Real(n) => Cell::new(n).fg(Color::Yellow),
        ValueRef::Text(text) => Cell::new(String::from_utf8_lossy(text)),
        ValueRef::Blob(blob) => {
            Cell::new(blob.iter().map(|byte| format!("{:02x}", byte)).join(" "))
        }
    }
}

impl OutputRows for TableOutput {
    fn begin(&mut self, columns: &[Column<'_>]) -> anyhow::Result<()> {
        let mut table = Table::new();
        table.load_preset("││──╞══╡│    ──┌┐└┘");
        table.set_header(columns.iter().map(|col| col.name()));
        self.table = Some(table);
        self.num_columns = columns.len();
        Ok(())
    }

    fn add_row(&mut self, row: &Row<'_>) -> anyhow::Result<()> {
        let table_row = (0..self.num_columns)
            // We are iterating over column_count() so this should never fail
            .map(|index| row.get_ref_unwrap(index))
            .map(value_to_cell);
        self.table.as_mut().unwrap().add_row(table_row);
        Ok(())
    }

    fn finish(&mut self) -> anyhow::Result<()> {
        let mut table = self.table.take().unwrap();
        table.set_content_arrangement(ContentArrangement::Dynamic);
        // TODO do this in a generic way for all output types
        if table.get_row(100).is_some() {
            let mut command = Command::new("less")
                .stdin(Stdio::piped())
                .env("LESSCHARSET", "UTF-8")
                .spawn()?;
            writeln!(command.stdin.as_mut().unwrap(), "{}", table)?;
            command.wait()?;
        } else {
            println!("{}", table);
        }
        Ok(())
    }
}

pub struct SQLOutput<'a> {
    pub table_name: String,
    pub highlighted: bool,
    pub highlighter: &'a SQLHighlighter,
    pub num_columns: usize,
}

impl<'a> SQLOutput<'a> {
    fn println(&self, sql: &str) {
        if self.highlighted {
            println!("{}", self.highlighter.highlight(sql).unwrap());
        } else {
            println!("{}", sql);
        }
    }
}

impl<'a> OutputRows for SQLOutput<'a> {
    fn begin(&mut self, columns: &[Column<'_>]) -> anyhow::Result<()> {
        self.num_columns = columns.len();
        Ok(())
    }
    fn add_row(&mut self, row: &Row<'_>) -> anyhow::Result<()> {
        let mut sql = format!("INSERT INTO {} VALUES(", &self.table_name);
        for index in 0..self.num_columns {
            use std::fmt::Write;
            if index > 0 {
                sql.push_str(", ");
            }
            match row.get_ref(index)? {
                ValueRef::Null => sql.push_str("NULL"),
                ValueRef::Integer(n) => write!(&mut sql, "{}", n).unwrap(),
                ValueRef::Real(n) => write!(&mut sql, "{}", n).unwrap(),
                ValueRef::Text(text) => {
                    write!(&mut sql, "'{}'", std::str::from_utf8(text).unwrap()).unwrap()
                }
                ValueRef::Blob(blob) => write!(
                    &mut sql,
                    "X'{}'",
                    blob.iter()
                        .map(|byte| format!("{:02x}", byte))
                        .collect::<String>()
                )
                .unwrap(),
            }
        }
        sql.push_str(");");
        self.println(&sql);
        Ok(())
    }
    fn finish(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
}
