// Based on https://docs.rs/once_cell/1.8.0/once_cell/#lazily-compiled-regex
macro_rules! tree_sitter_query {
    ($query:literal $(,)?) => {{
        use once_cell::sync::OnceCell;
        use tree_sitter::Query;

        static QUERY: OnceCell<Query> = OnceCell::new();
        QUERY.get_or_init(|| Query::new(tree_sitter_sqlite::language(), $query).unwrap())
    }};
}
