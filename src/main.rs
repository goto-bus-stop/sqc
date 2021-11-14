use clap::Parser;
use comfy_table::{Cell, Color, ContentArrangement, Table};
use itertools::Itertools;
use rusqlite::types::ValueRef;
use rusqlite::Connection;
use rustyline::error::ReadlineError;
use rustyline::Editor;
use std::path::PathBuf;

mod format;
mod input;

use format::highlight_sql;
use input::EditorHelper;

fn value_to_cell(value: ValueRef) -> Cell {
    match value {
        ValueRef::Null => Cell::new("NULL"),
        ValueRef::Integer(n) => Cell::new(n).fg(Color::Yellow),
        ValueRef::Real(n) => Cell::new(n).fg(Color::Yellow),
        ValueRef::Text(text) => Cell::new(String::from_utf8_lossy(text)),
        ValueRef::Blob(blob) => {
            Cell::new(blob.iter().map(|byte| format!("{:02x}", byte)).join(" "))
        }
    }
}

fn print_sql(sql: &str) -> anyhow::Result<()> {
    let formatted = sqlformat::format(sql, &Default::default(), Default::default());
    let highlighted = highlight_sql(&formatted)?;
    println!("{}", highlighted);
    Ok(())
}

struct App {
    rl: Editor<EditorHelper>,
    conn: Connection,
}

impl App {
    fn run(&mut self) -> anyhow::Result<()> {
        let _ = self.rl.load_history("history.txt");
        let prompt = format!(
            "{}> ",
            self.rl.helper().unwrap().name().unwrap_or(":memory:")
        );
        loop {
            let readline = self.rl.readline(&prompt);
            match readline {
                Ok(line) => {
                    self.rl.add_history_entry(line.as_str());
                    if let Err(err) = self.execute(line.as_str()) {
                        println!("Error: {:?}", err);
                    }
                }
                Err(ReadlineError::Interrupted) => break,
                Err(ReadlineError::Eof) => break,
                Err(err) => {
                    println!("Error: {:?}", err);
                    break;
                }
            }
        }
        self.rl.save_history("history.txt")?;
        Ok(())
    }

    fn execute(&mut self, request: &str) -> anyhow::Result<()> {
        if request.starts_with(".tables") {
            self.execute_tables()
        } else if request.starts_with(".schema") {
            let mut parts = request.splitn(2, ' ').collect::<Vec<_>>();
            match parts.pop() {
                None | Some("") | Some(".schema") => anyhow::bail!("provide a table name"),
                Some(table_name) => self.execute_schema(table_name),
            }
        } else {
            self.execute_select_query(request)
        }
    }

    fn execute_tables(&mut self) -> anyhow::Result<()> {
        let mut stmt = self
            .conn
            .prepare("SELECT name FROM sqlite_schema WHERE type = 'table' ORDER BY name ASC")?;
        let tables = stmt.query_map([], |row| row.get::<_, String>(0))?;
        for table in tables {
            println!("{}", table?);
        }

        Ok(())
    }

    fn execute_schema(&mut self, table_name: &str) -> anyhow::Result<()> {
        let mut stmt = self
            .conn
            .prepare("SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = ?")?;
        let mut query = stmt.query([table_name])?;
        let row = if let Some(row) = query.next()? {
            row
        } else {
            anyhow::bail!("table {} does not exist", table_name);
        };

        let value_ref = row.get_ref(0)?;
        let sql = if let ValueRef::Text(text) = value_ref {
            std::str::from_utf8(text)?
        } else {
            anyhow::bail!("sqlite_schema table does not contain `text` for some reason?");
        };

        print_sql(sql)?;

        return Ok(());
    }

    fn execute_select_query(&mut self, sql: &str) -> anyhow::Result<()> {
        let mut stmt = self.conn.prepare(sql)?;
        if stmt.parameter_count() > 0 {
            anyhow::bail!("cannot run queries that require bind parameters");
        }

        let mut table = Table::new();
        table.load_preset("││──╞══╡│    ──┌┐└┘");
        table.set_header(stmt.column_names());

        let mut query = stmt.query([])?;
        while let Some(row) = query.next()? {
            let columns = 0..row.as_ref().column_count();
            let table_row = columns
                .map(|index| row.get_ref(index).map(value_to_cell))
                .collect::<Result<Vec<_>, _>>()?;
            table.add_row(table_row);
        }

        table.set_content_arrangement(ContentArrangement::Dynamic);
        println!("{}", table);
        Ok(())
    }
}

#[derive(Parser)]
struct Opts {
    filename: Option<PathBuf>,
}

fn main() -> anyhow::Result<()> {
    let opts = Opts::parse();
    let conn = match &opts.filename {
        Some(filename) => Connection::open(&filename)?,
        None => Connection::open_in_memory()?,
    };

    let mut rl = Editor::<EditorHelper>::new();
    rl.set_helper(Some(EditorHelper {
        name: opts
            .filename
            .and_then(|f| f.file_name().map(|os| os.to_string_lossy().to_string())),
    }));

    let mut app = App { rl, conn };
    app.run()?;

    Ok(())
}
