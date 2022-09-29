use crate::sql::{parse_sql, ParsedSql};
use rusqlite::Connection;
use std::borrow::Cow;
use std::collections::HashMap;
use std::rc::Rc;
use tree_sitter::{Node, QueryCursor, TextProvider};

fn text_provider(input: &str) -> impl TextProvider<'_> {
    |node: Node<'_>| std::iter::once(input[node.byte_range()].as_bytes())
}

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

#[derive(Debug)]
struct QueryNames<'a> {
    ctes: HashMap<&'a str, Vec<String>>,
    table_aliases: HashMap<&'a str, &'a str>,
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
            .prepare_cached("SELECT name FROM sqlite_schema WHERE type = 'table' ORDER BY name ASC")
            .unwrap();
        let tables = stmt
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<Vec<String>, _>>();

        tables.unwrap()
    }

    fn parse_names<'a>(&self, tree: &ParsedSql<'a>) -> QueryNames<'a> {
        let query = tree_sitter_query!("
            (with_clause (WITH) (common_table_expression (identifier) @cte-name (AS) (select_stmt) @cte)) @whole-cte
            (table_or_subquery (identifier) @table (identifier) @table-alias)
        ");
        let mut cursor = QueryCursor::new();
        let mut ctes = HashMap::new();
        let mut table_aliases = HashMap::new();
        let mut cte_prefix = String::new();
        for m in cursor.matches(query, tree.tree.root_node(), text_provider(tree.source)) {
            match m.pattern_index {
                0 => {
                    let mut column_names = vec![];
                    let whole = &tree.source[m.captures[0].node.byte_range()];
                    let expr = &tree.source[m.captures[2].node.byte_range()];
                    if let Ok(stmt) = self.connection.prepare(&format!("{} {}", cte_prefix, expr)) {
                        column_names = stmt
                            .column_names()
                            .into_iter()
                            .map(ToOwned::to_owned)
                            .collect::<Vec<_>>();
                    }
                    ctes.insert(&tree.source[m.captures[1].node.byte_range()], column_names);
                    if !cte_prefix.is_empty() {
                        cte_prefix += ", ";
                    }
                    cte_prefix += whole;
                }
                1 => {
                    if let [table, alias] = m.captures {
                        table_aliases.insert(
                            &tree.source[alias.node.byte_range()],
                            &tree.source[table.node.byte_range()],
                        );
                    }
                }
                _ => unreachable!(),
            }
        }
        QueryNames {
            ctes,
            table_aliases,
        }
    }

    pub fn get_completions(&self, sql: &str, pos: usize) -> Vec<(usize, String)> {
        let tree = parse_sql(sql).unwrap();
        let names = self.parse_names(&tree);
        let root = tree.tree.root_node();
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
                        let cte_names = names.ctes.keys().map(ToString::to_string);
                        let alias_names = names.table_aliases.keys().map(ToString::to_string);
                        return self
                            .get_table_names()
                            .into_iter()
                            .chain(cte_names)
                            .chain(alias_names)
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
