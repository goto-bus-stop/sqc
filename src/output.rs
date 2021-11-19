use crate::highlight::SQLHighlighter;
use comfy_table::{Cell, Color, ContentArrangement, Table};
use csv::{ByteRecord, Writer, WriterBuilder};
use itertools::Itertools;
use rusqlite::types::ValueRef;
use rusqlite::{Row, Statement};
use std::fs::File;
use std::io::Write;
use std::process::{Command, Stdio};
use std::str::FromStr;
use termcolor::{StandardStream, WriteColor};

pub enum OutputTarget {
    Stdout(StandardStream),
    File(File),
}

impl OutputTarget {
    pub fn start(&mut self) -> Box<dyn WriteColor + '_> {
        match self {
            OutputTarget::Stdout(stream) => Box::new(stream.lock()),
            OutputTarget::File(file) => Box::new(WriteColorFile(file)),
        }
    }
}

struct WriteColorFile<'f>(&'f mut File);
impl<'f> std::io::Write for WriteColorFile<'f> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.write(bytes)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}
impl<'f> WriteColor for WriteColorFile<'f> {
    fn supports_color(&self) -> bool {
        false
    }
    fn set_color(&mut self, _: &termcolor::ColorSpec) -> std::io::Result<()> {
        Ok(())
    }
    fn reset(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputMode {
    Null,
    Table,
    CSV,
    SQL,
}

impl OutputMode {
    pub fn output_rows<'h>(
        self,
        statement: &Statement<'_>,
        highlight: &'h SQLHighlighter,
        output: &'h mut dyn WriteColor,
    ) -> Box<dyn OutputRows + 'h> {
        match self {
            OutputMode::Null => Box::new(NullOutput),
            OutputMode::Table => Box::new(TableOutput::new(statement, output)),
            OutputMode::SQL => Box::new(SQLOutput::new(statement, highlight, output)),
            OutputMode::CSV => Box::new(CSVOutput::new(statement, output)),
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

pub struct TableOutput<'a> {
    table: Table,
    num_columns: usize,
    output: &'a mut dyn WriteColor,
}

impl<'a> TableOutput<'a> {
    pub fn new(statement: &Statement<'_>, output: &'a mut dyn WriteColor) -> Self {
        let mut table = Table::new();
        table.load_preset("││──╞══╡│    ──┌┐└┘");
        let names = statement.column_names();
        let num_columns = names.len();
        table.set_header(names);
        Self {
            table,
            num_columns,
            output,
        }
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

impl<'a> OutputRows for TableOutput<'a> {
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
            writeln!(self.output, "{}", self.table)?;
        }
        Ok(())
    }
}

pub struct CSVOutput<'a> {
    writer: Writer<&'a mut dyn WriteColor>,
}

impl<'a> CSVOutput<'a> {
    pub fn new(statement: &Statement<'_>, output: &'a mut dyn WriteColor) -> Self {
        let mut writer = WriterBuilder::new().has_headers(true).from_writer(output);

        // TODO return result
        writer
            .write_byte_record(&ByteRecord::from(statement.column_names()))
            .unwrap();

        Self { writer }
    }
}

impl<'a> OutputRows for CSVOutput<'a> {
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
    table_name: String,
    pub highlighted: bool,
    highlighter: &'a SQLHighlighter,
    output: &'a mut dyn WriteColor,
    num_columns: usize,
}

impl<'a> SQLOutput<'a> {
    pub fn new(
        statement: &Statement<'_>,
        highlighter: &'a SQLHighlighter,
        output: &'a mut dyn WriteColor,
    ) -> Self {
        let num_columns = statement.column_count();

        Self {
            table_name: "tbl".to_string(),
            // TODO make this depend on the output stream
            highlighted: true,
            highlighter,
            output,
            num_columns,
        }
    }

    pub fn with_table_name(self, table_name: String) -> Self {
        Self { table_name, ..self }
    }

    fn println(&mut self, sql: &str) -> std::io::Result<()> {
        if self.highlighted {
            writeln!(self.output, "{}", self.highlighter.highlight(sql).unwrap())
        } else {
            writeln!(self.output, "{}", sql)
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
        self.println(&sql)?;
        Ok(())
    }
    fn finish(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
}
