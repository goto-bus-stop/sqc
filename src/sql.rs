use tree_sitter::{Node, Parser, QueryCursor, TextProvider, Tree};

fn text_provider(input: &str) -> impl TextProvider<'_> {
    |node: Node<'_>| std::iter::once(input[node.byte_range()].as_bytes())
}

pub struct ParsedSql<'a> {
    pub source: &'a str,
    pub tree: Tree,
}

impl<'a> TextProvider<'a> for ParsedSql<'a> {
    type I = std::iter::Once<&'a [u8]>;

    fn text(&mut self, node: Node<'_>) -> Self::I {
        std::iter::once(self.source[node.byte_range()].as_bytes())
    }
}

impl<'a> ParsedSql<'a> {
    pub fn statements(&self) -> Vec<Node<'_>> {
        let statements_query = tree_sitter_query!("(sql_stmt_list (sql_stmt) @stmt)");
        let mut cursor = QueryCursor::new();
        let mut nodes = vec![];
        for stmt in cursor.matches(
            statements_query,
            self.tree.root_node(),
            text_provider(self.source),
        ) {
            nodes.push(stmt.captures[0].node);
        }
        nodes
    }
}

pub fn parse_sql(sql: &str) -> anyhow::Result<ParsedSql<'_>> {
    let mut parser = Parser::new();
    parser.set_language(tree_sitter_sqlite::language())?;
    let tree = parser.parse(sql, None).unwrap();
    Ok(ParsedSql { tree, source: sql })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_statements() {
        let tree = parse_sql("
            CREATE INDEX guess_user_id ON guesses(user_id);
            CREATE INDEX guess_round_id ON guesses(round_id);
            CREATE INDEX round_game_id ON rounds(game_id);
        ").unwrap();
        let statements = tree.statements();

        assert_eq!(
            statements.into_iter().map(|node| &tree.source[node.byte_range()]).collect::<Vec<_>>(),
            vec![
                "CREATE INDEX guess_user_id ON guesses(user_id)",
                "CREATE INDEX guess_round_id ON guesses(round_id)",
                "CREATE INDEX round_game_id ON rounds(game_id)",
            ],
        );
    }
}
