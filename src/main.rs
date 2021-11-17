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
mod highlight;
mod input;

use completions::Completions;
use input::EditorHelper;

// Based on https://docs.rs/once_cell/1.8.0/once_cell/#lazily-compiled-regex
macro_rules! tree_sitter_query {
    ($query:literal $(,)?) => {{
        static QUERY: once_cell::sync::OnceCell<tree_sitter::Query> =
            once_cell::sync::OnceCell::new();
        QUERY.get_or_init(|| {
            tree_sitter::Query::new(tree_sitter_sqlite::language(), $query).unwrap()
        })
    }};
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
        if request.starts_with(".") {
            let parts = request.splitn(2, ' ').collect::<Vec<_>>();
            match &parts[..] {
                [".tables"] => self.execute_tables(),
                [".schema"] => anyhow::bail!("provide a table name"),
                [".schema", table_name] => self.execute_schema(table_name),
                [".parse"] => anyhow::bail!("provide a query to parse"),
                [".parse", sql] => {
                    let tree = crate::highlight::parse_sql(sql)?;
                    println!("tree = {}", tree.root_node().to_sexp());
                    Ok(())
                }
                [".dump"] => self.execute_dump(None),
                [".dump", filter] => self.execute_dump(Some(filter)),
                _ => anyhow::bail!("unknown dot command"),
            }
        } else {
            let tree = crate::highlight::parse_sql(request)?;
            let statements_query = tree_sitter_query!("(sql_stmt_list (sql_stmt) @stmt)");
            let mut cursor = tree_sitter::QueryCursor::new();
            for stmt in cursor.matches(statements_query, tree.root_node(), text_provider(request)) {
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

    fn execute_dump(&mut self, filter: Option<&str>) -> anyhow::Result<()> {
        let mut tables_stmt = self.conn.prepare_cached(
            "SELECT name, sql FROM sqlite_schema WHERE type = 'table' AND tbl_name LIKE ?",
        )?;
        let mut tables_query = tables_stmt.query([filter
            .map(|name| format!("%{}%", name))
            .unwrap_or_else(|| "%".to_string())])?;
        let mut opt_row = if let Some(row) = tables_query.next()? {
            Some(row)
        } else {
            anyhow::bail!("no results for {}", filter.unwrap_or(""));
        };

        let highlighter = &self.helper().highlighter;
        println!("{}", highlighter.highlight("PRAGMA foreign_keys=OFF;")?);
        println!("{}", highlighter.highlight("BEGIN TRANSACTION;")?);

        while let Some(row) = opt_row {
            let name: String = row.get_unwrap(0);
            let sql: String = row.get_unwrap(1);

            let mut formatted = sqlformat::format(&sql, &Default::default(), Default::default());
            formatted.push(';');
            println!("{}", highlighter.highlight(&formatted)?);

            let mut rows_stmt = self.conn.prepare(&format!("SELECT * FROM {}", &name))?;
            let cols = rows_stmt.column_count();
            let mut rows_query = rows_stmt.query([])?;
            while let Some(row) = rows_query.next()? {
                let mut sql = format!("INSERT INTO {} VALUES(", &name);
                for index in 0..cols {
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
                println!("{}", highlighter.highlight(&sql)?);
            }

            opt_row = tables_query.next()?;
        }

        println!("{}", highlighter.highlight("COMMIT;")?);
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

    let mut rl = Editor::new();
    rl.set_helper(Some(EditorHelper::new(
        opts.filename
            .and_then(|f| f.file_name().map(|os| os.to_string_lossy().to_string())),
        completions,
    )));

    let mut app = App { rl, conn };
    app.run()?;

    Ok(())
}
