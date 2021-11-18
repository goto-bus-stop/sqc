use crate::highlight::SQLHighlighter;
use comfy_table::{Cell, Color, ContentArrangement, Table};
use csv::{ByteRecord, Writer, WriterBuilder};
use itertools::Itertools;
use rusqlite::types::ValueRef;
use rusqlite::{Row, Statement};
use std::io::Write;
use std::process::{Command, Stdio};
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputMode {
    Null,
    Table,
    CSV,
    SQL,
}

impl OutputMode {
    pub fn table<'h>(
        self,
        statement: &Statement<'_>,
        highlight: &'h SQLHighlighter,
    ) -> Box<dyn OutputRows + 'h> {
        match self {
            OutputMode::Null => Box::new(NullOutput),
            OutputMode::Table => Box::new(TableOutput::new(statement)),
            OutputMode::SQL => Box::new(SQLOutput::new(statement, highlight)),
            OutputMode::CSV => Box::new(CSVOutput::new(statement)),
        }
    }
}

impl FromStr for OutputMode {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "null" => Ok(Self::Null),
            "table" => Ok(Self::Table),
            "csv" => Ok(Self::CSV),
            "sql" => Ok(Self::SQL),
            _ => Err(()),
        }
    }
}

pub trait OutputRows {
    fn add_row(&mut self, row: &Row<'_>) -> anyhow::Result<()>;
    fn finish(&mut self) -> anyhow::Result<()>;
}

pub struct NullOutput;
impl OutputRows for NullOutput {
    fn add_row(&mut self, _row: &Row<'_>) -> anyhow::Result<()> {
        Ok(())
    }
    fn finish(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
}

#[derive(Default)]
pub struct TableOutput {
    table: Table,
    num_columns: usize,
}

impl TableOutput {
    pub fn new(statement: &Statement<'_>) -> Self {
        let mut table = Table::new();
        table.load_preset("││──╞══╡│    ──┌┐└┘");
        let names = statement.column_names();
        let num_columns = names.len();
        table.set_header(names);
        Self { table, num_columns }
    }
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
    fn add_row(&mut self, row: &Row<'_>) -> anyhow::Result<()> {
        let table_row = (0..self.num_columns)
            // We are iterating over column_count() so this should never fail
            .map(|index| row.get_ref_unwrap(index))
            .map(value_to_cell);
        self.table.add_row(table_row);
        Ok(())
    }

    fn finish(&mut self) -> anyhow::Result<()> {
        self.table
            .set_content_arrangement(ContentArrangement::Dynamic);
        // TODO do this in a generic way for all output types
        if self.table.get_row(100).is_some() {
            let mut command = Command::new("less")
                .stdin(Stdio::piped())
                .env("LESSCHARSET", "UTF-8")
                .spawn()?;
            writeln!(command.stdin.as_mut().unwrap(), "{}", self.table)?;
            command.wait()?;
        } else {
            println!("{}", self.table);
        }
        Ok(())
    }
}

pub struct CSVOutput {
    writer: Writer<Box<dyn Write>>,
}

impl CSVOutput {
    pub fn new(statement: &Statement<'_>) -> Self {
        let stdout = Box::new(std::io::stdout()) as Box<dyn Write>;
        let mut writer = WriterBuilder::new().has_headers(true).from_writer(stdout);

        // TODO return result
        writer
            .write_byte_record(&ByteRecord::from(statement.column_names()))
            .unwrap();

        Self { writer }
    }
}

impl OutputRows for CSVOutput {
    fn add_row(&mut self, row: &Row<'_>) -> anyhow::Result<()> {
        for index in 0..row.as_ref().column_count() {
            let val = row.get_ref_unwrap(index);
            match val {
                ValueRef::Null => self.writer.write_field([]),
                ValueRef::Integer(n) => self.writer.write_field(format!("{}", n)),
                ValueRef::Real(n) => self.writer.write_field(format!("{}", n)),
                ValueRef::Text(text) => self.writer.write_field(text),
                ValueRef::Blob(blob) => self.writer.write_field(blob),
            }?;
        }
        self.writer.write_record(None::<&[u8]>)?;
        Ok(())
    }

    fn finish(&mut self) -> anyhow::Result<()> {
        self.writer.flush()?;
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
    pub fn new(statement: &Statement<'_>, highlighter: &'a SQLHighlighter) -> Self {
        let num_columns = statement.column_count();

        Self {
            table_name: "tbl".to_string(),
            // TODO make this depend on the output stream
            highlighted: true,
            highlighter,
            num_columns,
        }
    }

    pub fn with_table_name(self, table_name: String) -> Self {
        Self { table_name, ..self }
    }

    fn println(&self, sql: &str) {
        if self.highlighted {
            println!("{}", self.highlighter.highlight(sql).unwrap());
        } else {
            println!("{}", sql);
        }
    }
}

impl<'a> OutputRows for SQLOutput<'a> {
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
