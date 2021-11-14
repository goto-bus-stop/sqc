use clap::Parser;
use cli_table::format::{HorizontalLine, Separator, VerticalLine};
use cli_table::{print_stdout, Cell as _, Style as _, Table as _};
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

fn display_value_ref(value: ValueRef) -> String {
    match value {
        ValueRef::Null => "NULL".to_string(),
        ValueRef::Integer(n) => n.to_string(),
        ValueRef::Real(n) => n.to_string(),
        ValueRef::Text(text) => String::from_utf8_lossy(text).to_string(),
        ValueRef::Blob(blob) => blob.iter().map(|byte| format!("{:x}", byte)).join(" "),
    }
}

fn print_sql(sql: &str) -> anyhow::Result<()> {
    let formatted = sqlformat::format(sql, &Default::default(), Default::default());
    let highlighted = highlight_sql(&formatted)?;
    println!("{}", highlighted);
    Ok(())
}

#[derive(Parser)]
struct Opts {
    filename: PathBuf,
}

struct App {
    rl: Editor<EditorHelper>,
    conn: Connection,
}

impl App {
    fn run(&mut self) -> anyhow::Result<()> {
        let _ = self.rl.load_history("history.txt");
        loop {
            let readline = self.rl.readline(">> ");
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

        let title: Vec<_> = stmt
            .columns()
            .iter()
            .map(|column| column.name().cell().bold(true))
            .collect();
        let mut results: Vec<Vec<String>> = vec![];
        let mut query = stmt.query([])?;
        while let Some(row) = query.next()? {
            let columns = 0..row.as_ref().column_count();
            let table_row = columns
                .map(|index| row.get_ref(index).map(display_value_ref))
                .collect::<Result<_, _>>()?;
            results.push(table_row);
        }
        print_stdout(
            results
                .table()
                .separator(
                    Separator::builder()
                        .title(Some(HorizontalLine::default()))
                        .column(Some(VerticalLine::default()))
                        .row(None)
                        .build(),
                )
                .title(title),
        )?;
        Ok(())
    }
}

fn main() -> anyhow::Result<()> {
    let opts = Opts::parse();
    let conn = Connection::open(&opts.filename)?;
    // `()` can be used when no completer is required
    let mut rl = Editor::<EditorHelper>::new();
    rl.set_helper(Some(EditorHelper {}));

    let mut app = App { rl, conn };
    app.run()?;

    Ok(())
}
