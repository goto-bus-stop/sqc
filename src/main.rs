use clap::{CommandFactory as _, Parser};
use directories::ProjectDirs;
use rusqlite::types::ValueRef;
use rusqlite::Connection;
use rustyline::error::ReadlineError;
use rustyline::Editor;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use termcolor::{ColorChoice, StandardStream};

#[macro_use]
mod macros;
mod completions;
mod highlight;
mod input;
mod output;
mod sql;

use completions::Completions;
use input::EditorHelper;
use output::{OutputMode, OutputRows, OutputTarget, SqlOutput};

/// Helper enum to take in "on"/"off" strings and turn them into bool true/false.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum OnOff {
    On,
    Off,
}
impl From<OnOff> for bool {
    fn from(v: OnOff) -> bool {
        v == OnOff::On
    }
}

#[derive(Debug, Clone, Parser)]
#[command(
    no_binary_name = true,
    disable_help_subcommand = true,
    help_template = "{all-args}"
)]
enum DotCommand {
    /// Print this message or the help of the given subcommand(s).
    #[command(name = ".help")]
    Help { subcommand: Option<String> },
    /// Print names of all tables in the database.
    #[command(name = ".tables")]
    Tables,
    /// Turn command echo on or off.
    #[command(name = ".echo")]
    Echo { enabled: OnOff },
    /// Set the output format/mode.
    #[command(name = ".mode")]
    Mode {
        #[arg(value_enum)]
        output_mode: OutputMode,
    },
    /// Send output to a file, or stdout.
    #[command(name = ".output")]
    Output { filename: Option<PathBuf> },
    /// Print the schema for a table.
    #[command(name = ".schema")]
    Schema { table_name: String },
    /// Print the parse tree for an SQL statement.
    #[command(name = ".parse")]
    Parse { sql: String },
    /// Print database content as SQL statements.
    #[command(name = ".dump")]
    Dump { filter: Option<String> },
    /// Create a full backup of a running database.
    #[command(name = ".backup")]
    Backup { filename: PathBuf },
}

struct App {
    rl: Editor<EditorHelper>,
    conn: Rc<Connection>,
    output_target: OutputTarget,
    output_mode: OutputMode,
    echo: bool,
}

impl App {
    fn run(&mut self) -> anyhow::Result<()> {
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
        Ok(())
    }

    fn execute(&mut self, request: &str) -> anyhow::Result<()> {
        if request.starts_with('.') {
            self.execute_dot_command(request)
        } else {
            if self.echo {
                let formatted = sqlformat::format(request, &Default::default(), Default::default());
                let mut output = self.output_target.start();
                let highlighter = &self.rl.helper().unwrap().highlighter;
                let highlighted = if output.supports_color() {
                    highlighter.highlight(&formatted)?
                } else {
                    formatted
                };
                writeln!(&mut output, "{}", highlighted)?;
            }

            // A single input may contain multiple SQL statements. Parse them
            // out and execute individually.
            let tree = crate::sql::parse_sql(request)?;
            for stmt_node in tree.statements() {
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

    fn execute_dot_command(&mut self, request: &str) -> anyhow::Result<()> {
        let clap_args = request.splitn(2, ' ');

        match DotCommand::try_parse_from(clap_args) {
            Ok(DotCommand::Help { subcommand: None }) => {
                DotCommand::command().disable_help_flag(true).print_help()?;
                Ok(())
            }
            Ok(DotCommand::Help {
                subcommand: Some(name),
            }) => {
                // make sure it has a . in front
                let name = format!(".{}", name.strip_prefix('.').unwrap_or(&name));

                if let Some(subcommand) = DotCommand::command().find_subcommand_mut(name) {
                    subcommand.print_help()?;
                } else {
                    DotCommand::command()
                        .disable_help_subcommand(true)
                        .print_help()?;
                }
                Ok(())
            }
            Ok(DotCommand::Tables) => self.execute_tables(),
            Ok(DotCommand::Echo { enabled }) => {
                self.echo = enabled.into();
                Ok(())
            }
            Ok(DotCommand::Mode { output_mode }) => {
                self.output_mode = output_mode;
                Ok(())
            }
            Ok(DotCommand::Output { filename: None }) => {
                self.output_target =
                    OutputTarget::Stdout(StandardStream::stdout(ColorChoice::Auto));
                Ok(())
            }
            Ok(DotCommand::Output {
                filename: Some(filename),
            }) => {
                self.output_target = OutputTarget::File(std::fs::File::create(filename)?);
                Ok(())
            }
            Ok(DotCommand::Schema { table_name }) => self.execute_schema(&table_name),
            Ok(DotCommand::Parse { sql }) => {
                let tree = crate::sql::parse_sql(&sql)?;
                writeln!(
                    self.output_target.start(),
                    "{}",
                    tree.tree.root_node().to_sexp()
                )?;
                Ok(())
            }
            Ok(DotCommand::Dump { filter }) => self.execute_dump(filter.as_deref()),
            Ok(DotCommand::Backup { filename }) => self.execute_backup(&filename),
            Err(err) => {
                err.print()?;
                Ok(())
            }
        }
    }

    /// Execute a .tables command.
    fn execute_tables(&mut self) -> anyhow::Result<()> {
        let mut output = self.output_target.start();

        let mut stmt = self
            .conn
            .prepare("SELECT name FROM sqlite_schema WHERE type = 'table' ORDER BY name ASC")?;
        let tables = stmt.query_map([], |row| row.get::<_, String>(0))?;
        for table in tables {
            writeln!(&mut output, "{}", table?)?;
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

        let highlighter = &self.rl.helper().unwrap().highlighter;
        let formatted = sqlformat::format(sql, &Default::default(), Default::default());

        let mut output = self.output_target.start();
        let highlighted = if output.supports_color() {
            highlighter.highlight(&formatted)?
        } else {
            formatted
        };
        writeln!(&mut output, "{}", highlighted)?;

        Ok(())
    }

    fn execute_dump(&mut self, filter: Option<&str>) -> anyhow::Result<()> {
        let mut output = self.output_target.start();

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

        let highlighter = &self.rl.helper().unwrap().highlighter;
        writeln!(
            &mut output,
            "{}",
            highlighter.highlight("PRAGMA foreign_keys=OFF;")?
        )?;
        writeln!(
            &mut output,
            "{}",
            highlighter.highlight("BEGIN TRANSACTION;")?
        )?;

        while let Some(row) = opt_row {
            let name: String = row.get_unwrap(0);
            let sql: String = row.get_unwrap(1);

            let mut formatted = sqlformat::format(&sql, &Default::default(), Default::default());
            formatted.push(';');
            writeln!(&mut output, "{}", highlighter.highlight(&formatted)?)?;

            let mut rows_stmt = self.conn.prepare(&format!("SELECT * FROM {}", &name))?;

            let mut output_rows =
                SqlOutput::new(&rows_stmt, highlighter, &mut output).with_table_name(name);

            let mut rows_query = rows_stmt.query([])?;
            while let Some(row) = rows_query.next()? {
                output_rows.add_row(row)?;
            }
            output_rows.finish()?;

            opt_row = tables_query.next()?;
        }

        writeln!(&mut output, "{}", highlighter.highlight("COMMIT;")?)?;
        Ok(())
    }

    fn execute_backup(&mut self, filename: &Path) -> anyhow::Result<()> {
        use indicatif::ProgressBar;
        use rusqlite::backup::{Backup, StepResult};

        let mut destination = Connection::open(filename)?;
        let backup = Backup::new(&self.conn, &mut destination)?;

        let bar = ProgressBar::new(0);
        while backup.step(100)? != StepResult::Done {
            let progress = backup.progress();
            bar.set_length(progress.pagecount.try_into().unwrap());
            bar.set_position((progress.pagecount - progress.remaining).try_into().unwrap());
        }

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

        let highlighter = &self.rl.helper().unwrap().highlighter;
        let mut output = self.output_target.start();
        let mut output_rows = self
            .output_mode
            .output_rows(&stmt, highlighter, &mut output);

        let mut query = stmt.query([])?;
        while let Some(row) = query.next()? {
            output_rows.add_row(row)?;
        }
        output_rows.finish()?;

        Ok(())
    }
}

#[derive(Parser)]
struct Opts {
    /// Filename of the database to open. If omitted, sqc opens a temporary in-memory database.
    filename: Option<PathBuf>,
    /// Queries to execute on the database. If omitted, sqc enters interactive mode.
    queries: Vec<String>,
}

fn main() -> anyhow::Result<()> {
    let opts = Opts::parse();
    let dirs = ProjectDirs::from("", "sqc", "sqc");
    let history_path = dirs
        .as_ref()
        .map(|dirs| dirs.data_dir().join("history.txt"));

    if let Some(dirs) = &dirs {
        let _ = std::fs::create_dir_all(dirs.data_dir());
    }

    let conn = Rc::new(match &opts.filename {
        Some(filename) => Connection::open(&filename)?,
        None => Connection::open_in_memory()?,
    });

    rusqlite::vtab::csvtab::load_module(&conn)?;

    let completions = Completions::new(Rc::clone(&conn));

    let mut rl = Editor::new()?;
    rl.set_helper(Some(EditorHelper::new(
        opts.filename
            .and_then(|f| f.file_name().map(|os| os.to_string_lossy().to_string())),
        completions,
    )));

    let mut app = App {
        rl,
        conn,
        output_target: OutputTarget::Stdout(StandardStream::stdout(ColorChoice::Auto)),
        output_mode: OutputMode::Table,
        echo: false,
    };

    if opts.queries.is_empty() {
        if let Some(path) = &history_path {
            let _ = app.rl.load_history(path);
        } else {
            eprintln!("Warning: could not load shell history: home directory not found");
        }
        app.run()?;
        if let Some(path) = &history_path {
            let _ = app.rl.save_history(path);
        }
    } else {
        for query in opts.queries {
            app.execute(&query)?;
        }
    }

    Ok(())
}
