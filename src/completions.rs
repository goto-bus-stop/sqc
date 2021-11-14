use crate::format::parse_sql;
use rusqlite::Connection;
use std::borrow::Cow;
use std::rc::Rc;

const INITIAL_KEYWORDS: [&str; 15] = [
    "SELECT", "DELETE", "CREATE", "DROP", "ATTACH", "DETACH", "EXPLAIN", "PRAGMA", "WITH",
    "UPDATE", "ALTER", "BEGIN", "END", "COMMIT", "ROLLBACK",
];

fn starts_with(item: &str, input: &str) -> bool {
    input.len() <= item.len() && item[..input.len()].eq_ignore_ascii_case(input)
}

fn match_case<'i>(item: &'i str, input: &str) -> Cow<'i, str> {
    if input.chars().all(|c| c.is_ascii_lowercase()) {
        item.to_ascii_lowercase().into()
    } else {
        item.into()
    }
}

pub struct Completions {
    connection: Rc<Connection>,
}

impl Completions {
    pub fn new(connection: Rc<Connection>) -> Self {
        Self { connection }
    }

    /// Maybe cache this later
    fn get_table_names(&self) -> Vec<String> {
        let mut stmt = self
            .connection
            .prepare("SELECT name FROM sqlite_schema WHERE type = 'table' ORDER BY name ASC")
            .unwrap();
        let tables = stmt
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<Vec<String>, _>>();

        tables.unwrap()
    }

    pub fn get_completions(&self, sql: &str, pos: usize) -> Vec<(usize, String)> {
        let tree = parse_sql(sql).unwrap();
        let root = tree.root_node();
        let max_lookbehind = 5.min(pos);
        let relevant_node = (0..max_lookbehind).find_map(|offset| {
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
                            .map(|item| {
                                (node.start_byte(), format!("{} ", match_case(item, content)))
                            })
                            .collect();
                    }
                    ("identifier", "table_or_subquery") => {
                        return self
                            .get_table_names()
                            .into_iter()
                            .filter(|item| starts_with(item, content))
                            .map(|item| (node.start_byte(), format!("{} ", item)))
                            .collect();
                    }
                    (_left, _right) => {
                        // Nice for debugging
                        // return vec![(node.end_byte(), format!("   -> {} {}", left, right))]
                    }
                },
                (Some(parent), Some(prev)) => match (node.kind(), parent.kind(), prev.kind()) {
                    (_left, _mid, _right) => {
                        // return vec![(node.end_byte(), format!("   -> {} {} {}", left, mid, right))]
                    }
                },
                (None, Some(_prev)) => (),
                (None, None) => (),
            }
        }

        Default::default()
    }
}
