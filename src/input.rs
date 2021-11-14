use rustyline::completion::Completer;
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::validate::Validator;
use rustyline::{Context, Helper};
use std::borrow::Cow;

use crate::format::{highlight_sql, parse_sql};

const INITIAL_KEYWORDS: [&'static str; 15] = [
    "SELECT", "DELETE", "CREATE", "DROP", "ATTACH", "DETACH", "EXPLAIN", "PRAGMA", "WITH",
    "UPDATE", "ALTER", "BEGIN", "END", "COMMIT", "ROLLBACK",
];
const TEST_TABLE_NAMES: [&'static str; 12] = [
    "_sqlx_migrations",
    "api_keys",
    "downloaded_images",
    "job_queue",
    "job_queue_old",
    "lastfm_import_jobs",
    "listens",
    "raw_tracks",
    "scrobbles",
    "sqlite_sequence",
    "tracks",
    "users",
];

fn starts_with(item: &str, input: &str) -> bool {
    input.len() <= item.len() && item[..input.len()].eq_ignore_ascii_case(input)
}

fn completions(sql: &str, pos: usize) -> Vec<(usize, String)> {
    let tree = parse_sql(sql).unwrap();
    let root = tree.root_node();
    let relevant_node = (0..5).find_map(|offset| {
        if let Some(node) = root.descendant_for_byte_range(pos - offset, pos - offset) {
            if node.kind() == "sql_stmt_list" {
                None
            } else {
                Some(node)
            }
        } else {
            None
        }
    });

    if let Some(node) = relevant_node {
        let parent = node.parent();
        let prev = node.prev_sibling();
        let content = &sql[node.byte_range()];

        // Handle the start of a statement.
        match (parent, prev) {
            (Some(parent), None) => match (node.kind(), parent.kind()) {
                ("ERROR", "sql_stmt_list") => {
                    return INITIAL_KEYWORDS
                        .into_iter()
                        .filter(|item| starts_with(item, content))
                        .map(|item| (node.start_byte(), format!("{} ", item)))
                        .collect();
                }
                ("identifier", "table_or_subquery") => {
                    return TEST_TABLE_NAMES
                        .into_iter()
                        .filter(|item| starts_with(item, content))
                        .map(|item| (node.start_byte(), format!("{} ", item)))
                        .collect();
                }
                (left, right) => {
                    // Nice for debugging
                    // return vec![(node.end_byte(), format!("   -> {} {}", left, right))]
                }
            },
            (Some(parent), Some(prev)) => match (node.kind(), parent.kind(), prev.kind()) {
                (left, mid, right) => {
                    // return vec![(node.end_byte(), format!("   -> {} {} {}", left, mid, right))]
                }
            },
            (None, Some(_prev)) => (),
            (None, None) => (),
        }
    }

    Default::default()
}

pub struct EditorHelper {
    pub name: Option<String>,
}

impl EditorHelper {
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }
}

impl Highlighter for EditorHelper {
    fn highlight<'l>(&self, line: &'l str, _pos: usize) -> Cow<'l, str> {
        match highlight_sql(line) {
            Ok(highlighted) => highlighted.into(),
            Err(_) => line.into(),
        }
    }

    fn highlight_hint<'h>(&self, hint: &'h str) -> Cow<'h, str> {
        format!("{}", hint).into()
    }

    fn highlight_char(&self, line: &str, _pos: usize) -> bool {
        !line.starts_with('.')
    }
}

impl Completer for EditorHelper {
    type Candidate = String;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Self::Candidate>)> {
        let results = completions(line, pos);
        if let Some(first) = results.get(0) {
            Ok((first.0, results.into_iter().map(|item| item.1).collect()))
        } else {
            Ok((0, vec![]))
        }
    }
}

impl Hinter for EditorHelper {
    type Hint = String;

    fn hint(&self, line: &str, pos: usize, _ctx: &Context<'_>) -> Option<Self::Hint> {
        completions(line, pos)
            .into_iter()
            .next()
            .map(|mut item| item.1.split_off(pos - item.0))
    }
}

impl Validator for EditorHelper {}

impl Helper for EditorHelper {}
