use clap::Parser;
use comfy_table::{Cell, Color, ContentArrangement, Table};
use itertools::Itertools;
use rusqlite::types::ValueRef;
use rusqlite::Connection;
use rustyline::error::ReadlineError;
use rustyline::Editor;
use std::path::PathBuf;
use std::rc::Rc;

mod completions;
mod format;
mod input;

use completions::Completions;
use input::EditorHelper;

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

fn text_provider(input: &str) -> impl tree_sitter::TextProvider<'_> {
    |node: tree_sitter::Node<'_>| std::iter::once(input[node.byte_range()].as_bytes())
}

struct App {
    rl: Editor<EditorHelper>,
    conn: Rc<Connection>,
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
                Err(ReadlineError::Interrupted) => continue,
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

    fn helper(&self) -> &EditorHelper {
        self.rl.helper().unwrap()
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
        } else if request.starts_with(".parse") {
            let parts = request.splitn(2, ' ').collect::<Vec<_>>();
            if parts.len() < 2 {
                anyhow::bail!("provide a query to parse");
            }
            let tree = crate::format::parse_sql(parts[1])?;
            println!("tree = {}", tree.root_node().to_sexp());
            Ok(())
        } else {
            use tree_sitter::{Query, QueryCursor};

            let tree = crate::format::parse_sql(request)?;
            let statements_query = Query::new(
                tree_sitter_sqlite::language(),
                "(sql_stmt_list (sql_stmt) @stmt)",
            )
            .unwrap();
            let mut cursor = QueryCursor::new();
            for stmt in cursor.matches(&statements_query, tree.root_node(), text_provider(request))
            {
                let stmt_node = stmt.captures[0].node;
                let sql = &request[stmt_node.byte_range()];
                let kind = stmt_node.child(0).map(|node| node.kind());
                match kind {
                    Some("update_stmt" | "delete_stmt" | "insert_stmt") => {
                        self.execute_update_query(sql)?
                    }
                    Some(
                        "create_index_stmt"
                        | "create_table_stmt"
                        | "create_trigger_stmt"
                        | "create_view_stmt"
                        | "create_virtual_table_stmt"
                        | "drop_index_stmt"
                        | "drop_table_stmt"
                        | "drop_trigger_stmt"
                        | "drop_view_stmt",
                    ) => self.execute_silent_query(sql)?,
                    Some(_) | None => self.execute_select_query(sql)?,
                }
            }
            Ok(())
        }
    }

    /// Execute a .tables command.
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

    /// Execute a .schema command.
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

        let highlighter = &self.helper().highlighter;
        let formatted = sqlformat::format(sql, &Default::default(), Default::default());
        let highlighted = highlighter.highlight(&formatted)?;
        println!("{}", highlighted);

        Ok(())
    }

    /// Execute an UPDATE, DELETE or INSERT query.
    fn execute_update_query(&mut self, sql: &str) -> anyhow::Result<()> {
        let mut stmt = self.conn.prepare(sql)?;
        if stmt.parameter_count() > 0 {
            anyhow::bail!("cannot run queries that require bind parameters");
        }

        let changes = stmt.execute([])?;
        println!("{} changes", changes);

        Ok(())
    }

    /// Execute a query that does not return anything.
    fn execute_silent_query(&mut self, sql: &str) -> anyhow::Result<()> {
        let mut stmt = self.conn.prepare(sql)?;
        if stmt.parameter_count() > 0 {
            anyhow::bail!("cannot run queries that require bind parameters");
        }

        let _ = stmt.execute([])?;

        Ok(())
    }

    /// Execute a SELECT query.
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
                // We are iterating over column_count() so this should never fail
                .map(|index| row.get_ref_unwrap(index))
                .map(value_to_cell);
            table.add_row(table_row);
        }

        table.set_content_arrangement(ContentArrangement::Dynamic);

        if table.get_row(100).is_some() {
            use std::io::Write;
            use std::process::{Command, Stdio};
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

#[derive(Parser)]
struct Opts {
    filename: Option<PathBuf>,
}

fn main() -> anyhow::Result<()> {
    let opts = Opts::parse();
    let conn = Rc::new(match &opts.filename {
        Some(filename) => Connection::open(&filename)?,
        None => Connection::open_in_memory()?,
    });

    let completions = Completions::new(Rc::clone(&conn));

    let mut rl = Editor::<EditorHelper>::new();
    rl.set_helper(Some(EditorHelper::new(
        opts.filename
            .and_then(|f| f.file_name().map(|os| os.to_string_lossy().to_string())),
        completions,
    )));

    let mut app = App { rl, conn };
    app.run()?;

    Ok(())
}
